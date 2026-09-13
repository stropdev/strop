# 0061 — Final GUI arc: native Windows presentation, WSL editor engine

Status: requested architectural/product/testing handoff; **not GUI implementation
permission for the current TUI release**. The user wants the GUI after the desired
TUI features and bug-hardening, with the entire supported editor experience—not
an empty window, text-editor subset or independent second editor.

This refines [0038](0038-remote-experience-and-responsiveness.md)'s GPUI direction
and [0055](0055-embedded-terminal-tui-and-gui.md)'s later GUI terminal milestone.
[0060](0060-debugger-workflow-and-architecture.md) owns debugger semantics;
[0062](0062-distribution-and-wsl-onboarding.md) owns packaging and onboarding.

**Entry gate:** [0056](0056-architecture-prerequisites.md) AR01–AR16,
[0057](0057-core-verification-and-assurance.md) VF01–VF20,
[0058](0058-unified-native-worker.md) WK01–WK20, 0059 completion and 0060 debugger/
TUI ledgers are complete first. Shared architecture, baseline verification and the
native-worker assurance cutover are preceding releases, not GUI implementation work.
UI01–UI18 still prove integration and full behavior over that requalified core.

## 1. Decision: keep GPUI, keep all workspace execution in WSL

**Ship a native Windows GPUI frontend backed by the existing Linux engine in a
selected WSL distribution.** Windows owns windows, GPU drawing, fonts, native
input/IME, clipboard and accessibility. WSL owns documents, grammar, syntax/LSP,
Git, DAP, filesystem operations, SSH/container connections and PTYs.

This is not a native Windows filesystem/process port. The user's projects remain
under Linux paths such as `/home/user/project`; native `C:\` workspaces, Windows
LSP/debugger processes and ConPTY are separate later work. Access through `/mnt/c`
is initially an explicitly limited Linux-side capability (§8), not a shortcut to
claiming full Windows filesystem semantics.

The versioned engine/view/input protocol and non-graphical backend are delivered
by 0056. This release adds the native Windows frontend and its transport/window
integration over that proven boundary. It keeps the editor/services in WSL without
forking semantics; missing generic contracts are prerequisite failures, not work
to hide inside a GUI callback.

Use `gpui` + the Windows platform implementation through `gpui_platform`, pinned
together to a tested upstream revision. Do not import Zed's editor/workspace/debugger
or terminal_view crates. GPUI's Apache-2.0 boundary is distinct from Zed's GPL app.
A private `strop-gui` package can ship binary artifacts without forcing unpublished
GPUI platform crates into Strop's published TUI dependency graph.

### Why not WSLg as the primary release?

A single Linux GUI process under WSLg would avoid IPC and is a useful development
option. It is not equivalent to a native Win32 frontend: graphics, IME, clipboard,
accessibility and window integration cross Wayland/X11/RDP boundaries.

Actual environment evidence here:

- Ubuntu WSL2 exposed DISPLAY=:0, WAYLAND_DISPLAY=wayland-0 and WSLg 1.0.73.2.
- A read-only Vulkan loader probe succeeded but enumerated only **llvmpipe**, a CPU
  device. This is not proof that hardware acceleration is impossible on WSL, but
  it is proof that the inspected Vulkan path was software-only.
- Microsoft's WSLg documentation describes accelerated OpenGL through Mesa/D3D12;
  that is not a guarantee that every GPUI/Vulkan path is ready on a user's distro.
- Current GPUI Linux sources use gpui_wgpu for Wayland/X11; do not repeat stale
  assumptions about older renderers. Windows sources have DirectX/DirectWrite
  implementations and an AccessKit Windows adapter.
- WSLg's documented display/application integration does not establish a Windows
  UI Automation projection of every Linux accessibility tree. Do not claim
  Narrator/IME parity without actual evidence.

Zed's supported native Windows→WSL workflow is the useful architectural precedent.
Strop deliberately differs by keeping **the authoritative editing engine itself**
in WSL rather than adding a second authoritative editing/syntax model on Windows.
No cloud broker or SSH server inside WSL is required for this local bridge.

### Framework gate, not framework loyalty

The inspected upstream ref was `d27fa556ce1e5aa9b606357b89a0841c9752f5ab`; this is
research provenance, not an instruction to use that revision forever. Pin the
selected GPUI/platform/font dependencies and prove native input, text, accessibility,
GPU capture, packaging and the bridge before broad UI work.

If GPUI fails a required gate, compare Iced/egui against the **same** vertical slice
and record a replacement decision before proceeding. Do not keep two GUI frameworks,
import GPL application code, ship a webview approximation as the agreed native GUI,
or weaken an acceptance criterion to protect the original toolkit choice.

## 2. Entry conditions and required release ledger

The GUI follows completion/hardening of the desired TUI arcs: 0051 R01–R11,
0053 S01–S10, 0054 F01–F12, 0055 T01–T10, **0056 AR01–AR16**, **0057 VF01–VF20**,
**0058 WK01–WK20**, **0059 C01–C09** and **0060 DBG01–DBG16**. Reconcile their evidence.
0046's readonly frontend boundary is closed by 0056, not by this GUI release.

The first GUI product release targets **Windows 11 x64 + supported WSL2 Linux
workspace execution**, not every desktop OS at once. Other native GUI platforms,
Windows ARM64 and native Windows workspaces have explicit later gates. The existing
Linux/macOS TUI distribution remains supported and independently buildable.

| ID | Required GUI deliverable |
| --- | --- |
| UI01 | Native Windows GPUI application with a pinned, tested framework/platform dependency boundary and no GUI dependencies in the TUI engine. |
| UI02 | Clean WSL selection/bootstrap/version negotiation and a functioning first-open journey; no hidden native Windows workspace fallback. |
| UI03 | Consume the released 0056 action/view/input contracts without a second editor, mutable frontend bypass or clone-per-frame state. |
| UI04 | Integrate the native Windows bridge/cache with the proven 0056 protocol/server, ordered outcomes, shutdown and draft-recovery policy. |
| UI05 | Complete parity matrix in §4: every supported TUI action/surface has a real GUI path and evidence. |
| UI06 | Native keyboard/IME/paste/pointer/selection routing, correct byte/UTF-16/cell/pixel boundaries and retained Vim semantics. |
| UI07 | Polished responsive typography/chrome/components, multi-pane identity, readable source/metadata/diff hierarchy and useful native window behavior. |
| UI08 | Full LSP/completion/diagnostic/navigation and source-backed search/collection/review behavior over the same semantic owners. |
| UI09 | Filesystem, SSH/container and namespace/capability parity, including protected mutations and honest mounted-Windows limits. |
| UI10 | Full 0060 debugger workflow and debugger presentation, not just breakpoint glyphs. |
| UI11 | Full 0055 GUI terminal surface over WSL-owned emulator/PTY sessions, including native input and accessibility. |
| UI12 | Native UI Automation/AccessKit text, selection, tree/actions and focus behavior, tested with real accessibility clients. |
| UI13 | Agent-drivable, opt-in private automation plus real native input and GPU capture; screenshots correlate with applied/presented revisions. |
| UI14 | Layered deterministic engine/frontend tests and actual Windows+WSL integration/hardware lanes. |
| UI15 | Measured input-to-present latency, bounded queues/memory/cache behavior and no TUI performance regression. |
| UI16 | Safe multiple windows/views, ownership-aware close/restart and no duplicated or orphaned backend/services. |
| UI17 | Signed clean installation/update/rollback/uninstall and truthful website/channel presentation under 0062. |
| UI18 | Complete parity/evidence ledger and docs, required protocol proof/conformance plus native GUI gates, and no unimplemented or unverified release placeholders. |

All UI requirements are release requirements. Internal vertical slices are not
smaller public “GUI complete” milestones. Omitting a supported TUI feature or moving
a UI requirement out requires explicit user approval and a roadmap record.

## 3. First-open and everyday user journeys

### First launch

1. Install from strop.dev's signed per-user Windows installer. Start-menu/installed-
   apps integration launches the native window; no terminal command is required.
2. Detect WSL availability without changing it. If missing/unsupported, explain the
   prerequisite and offer the explicit setup path from 0062. Never silently enable
   Windows features, install a distro, change defaults or run wsl --shutdown.
3. Choose an installed WSL distribution/user. Remember the successful choice, not
   an automatic privilege escalation. Show the exact distribution and Linux home.
4. Validate/install the matching backend privately under that Linux user with
   explicit first-use consent. This is the fixed Strop backend, not project code.
5. Complete protocol/version/capability negotiation before showing Ready. Failure
   names the actual stage: WSL unavailable, backend missing, mismatch, permission,
   failed launch or malformed handshake.
6. Open a WSL workspace through the shared file/directory picker. The default is
   Linux home/project storage, not C:\Users and not a recursive scan of `/`.
7. Read the existing WSL editor configuration and expose the same trust prompts.
   Do not duplicate language-server/debugger/SSH configuration on Windows.

### Daily work

Open source → edit/preview/undo → search/collect/replace review → filesystem
organization → Git/source inspection → debug → terminal → save/close uses the
same state transitions and authority as the TUI. GUI menus, clicks and shortcuts
are additional routes into those commands, not new implementations.

A workspace title/status makes **Ubuntu · /home/...** or **SSH via Ubuntu · host**
visible. Debugger/terminal titles cannot overwrite that trusted provenance. A
failure in a remote service is not a failure of the Windows renderer, and a lost
WSL bridge is not a successful save/stop.

## 4. Mandatory feature parity matrix

Freeze the final supported TUI action/capability inventory at implementation start,
then update it when TUI work lands. Generate coverage checks from the common command
registry where possible; a hand-selected “important features” sample is insufficient.
GUI parity includes the same supported refusals/limits—not invented capabilities.

| Family | Required GUI behavior/evidence |
| --- | --- |
| Modal editing | Normal/Insert/all Visual forms, counts/operators/text objects/surround, registers, marks, repeats/macros, search motions, operator preview and cancellation match the same pure resolver. |
| Text and coordinates | Unicode, CRLF, tabs, long lines, block selections, multicursors, occurrence selection, indentation inference/controls, pair matching and source syntax/injections remain correct. |
| Documents | Open/reveal/close, readonly/dirty state, save/save-as where supported, external changes, source identity, MRU, splits, viewport/jump/return state and local session behavior. |
| History | Undo/redo/tree browser, grouped edits, collection/source history and failed preflight/partial outcomes do not become GUI-only mutations. |
| LSP | Hover/Markdown, definitions/declarations/types/implementations/references, symbols, rename/code actions, diagnostics, external-header routing, workspace trust and remote contexts. |
| Completion | 0059's manual/automatic/disabled policies, popup navigation/cancellation, edits/imports/resolve, source-aware contexts and stale-response rules—not a separate toolkit autocomplete. |
| Files and Search | Quick finder plus 0053's large Search workspace, query parser/highlighting/suggestions, hidden/ignored filters, replacement toggle, retained investigation and included workset. |
| Changes and collections | Exact preview/review/Apply/Save, receipts/conflicts, source-backed live edits, multi-file cards/line numbers, g-Space/source returns and authoritative dirty buffers. |
| Filesystem | 0054 Directory browsing, completion, creation, rename/move/copy/Trash/removal, modal filename drafts, review/receipts and live-document relocation. |
| Git | Supported gutters/hunks/status, log/graph, commit-files/diff, blame, source navigation/permalinks and supported mutation/refusal behavior. |
| SSH | Host chooser/history, SFTP browse/read, full/range/tail/follow views, explicit edit/save/verify, remote Git/LSP and unchanged namespace/trust/lease guarantees. |
| Containers | All implemented attach/browse/service/debug capabilities with canonical container incarnation; no GUI-specific docker cp or host-path fallback. |
| Shell/tasks | Existing finite shell-output and pipe workflows remain useful and distinct from interactive terminals; owned cancellation and source-edit transactions remain intact. |
| Terminal | 0055 session/cursor/colors/scrollback/input/paste/selection/lifecycle/private capture; Linux PTY/emulation stays in WSL, Windows renders and handles native input. |
| Debugger | All 0060 launch/attach/build/breakpoint/exception/stack/variables/watch/evaluate/console/step/restart/detach/remote/container/session journeys. |
| Support surfaces | Command palette/help/which-key, fields/suggestions, settings/explain provenance, trust/progress/errors, update/onboarding states and all cancellation/focus returns. |
| Instrumentation | Engine replay/differential state, private trace/capture boundaries, diagnostics/bench data and GUI-specific presentation evidence. |

For every row record: shared owner, GUI component, keyboard route, pointer/accessibility
route where applicable, failure/readonly/cancel behavior and exercised artifacts.
Unavailable backend capabilities stay visibly unavailable for the same reason as
in the TUI. A disabled button is not proof of a feature required on a capable fixture.

## 5. Completed architecture and whole-core assurance entry contract

[0056](0056-architecture-prerequisites.md) owns the generic gaps identified in the
sweep: mutating render admission, broad frontend access, incomplete dirty-draft
recovery, event/service bounds and the non-graphical UI backend. Its AR01–AR16 ledger
closes before 0057 verifies the stable core, then completion/debugger extend it.

The GUI consumes:

- AR01–AR03: readonly prepared views, admitted input/actions, stable row/source
  identity and checked byte/UTF-16/logical geometry. Paint does not admit work.
- AR04–AR08: real recovery/watermarks, binding/outcome reconciliation, bounded
  lifecycle/exec, host-effect authority and privacy policy.
- AR09–AR10: working UI protocol/client state, `strop --ui-stdio`, semantic driver
  and real stdio/WSL lifecycle evidence.
- AR11–AR16: existing install/release identity, checked IDs, explain/build and
  integrated readiness contracts consumed by this release and 0062.
- **0057 VF01–VF20**, separately: the qualified pre-worker whole-core baseline and
  normative semantic/authority/effect contracts, calibrated proofs and required gates.
- **0058 WK01–WK20**: the same native worker protocol for local/SSH/container services,
  completed Python cutover, deployment/cache ownership and requalified production/
  editor/native correspondence. Baseline Python evidence is historical, not GUI authority.

GUI evidence still establishes the actual frontend respects them. A generic-contract
failure is repaired with the 0056/0057/0058 owners and its claims requalified; it does
not permit bypassing admission, adding recovery stores or a Windows-local editor.

The GUI is not a bitmap viewer for the TUI grid. Reuse semantic data/resolvers;
native fonts/pixels, chrome, fields, trees and accessible text remain this feature's
work. Borrow/cache bounded data and keep frontend caches non-authoritative.

## 6. Native frontend integration over the released backend

```text
Windows strop-gui                         implemented in this release
  GPUI / fonts / IME / clipboard / UIA / native-window ownership
  readonly view cache + native input/transport adapter
                  |
       owned wsl.exe stdio connection
                  |
released 0056 strop --ui-stdio             consumed, not invented here
  authoritative strop-engine / protocol / recovery / services
       local Linux FS, SSH/container contexts, LSP/Git/DAP, PTYs
```

One GUI workspace window initially owns one backend Editor session. Reopening the
same workspace normally reveals that window instead of accidentally starting a
second editor against the same files. Explicit independent windows retain normal
external-change/conflict safeguards; this is not a collaborative document daemon.

New code lives in the private **strop-gui** package: app/window, Windows process/
transport adaptation, readonly cache, layout/input, components/source surface,
debugger/terminal views, accessibility, capture/automation and onboarding integration.
The existing **strop-ui-protocol**, engine view producer and UI server come from
0056. Do not create another schema, protocol client reducer or server composition root.

Use the released handshake, action sequence/acknowledgement, view revision/delta,
viewport interest, effect and shutdown APIs. Native launch supplies the exact
distro/user/verified backend path; it must not inherit a C:\ workspace, open a
public port or invoke a login shell with unframed banners.

The Windows adapter preserves ordered input and effect identity across actual
stdio partial reads/writes and errors. It never coalesces editing keys, paste or
side effects. Safe presentation coalescing/resynchronization follows AR09's base
revision rules; unknown/stale data cannot become a pointer/edit target.

The Windows cache remains readonly. No local undo/edit engine, optimistic replay
after disconnect or whole-Editor clone. Render the latest valid view while native
input is queued; don't pretend an unacknowledged edit is saved. Offscreen text,
accessibility data, effects and retained rows obey the released bounds.

This is real native integration work and needs the Windows evidence below, but
not a repeat of the shared architecture release. No WSLg, Linux GUI stack, SSH
daemon, systemd service or native Windows workspace backend is required.

## 7. Integrate recovery and lifecycle, do not implement them again

0056 AR04–AR10 provide the recovery, outcome, liveness and stdio-close contracts.
The native window must use them: ask the backend about dirty drafts, pending
mutations and live services, display its choices/progress and await an authorized
shutdown outcome before releasing process ownership. Window close is not an
unconditional TerminateProcess(wsl.exe).

On native bridge loss, mark cached views disconnected/stale and stop admitting
actions. Preserve known acknowledgements and expose the released recovery/reopen
flow. Do not repeat a save, rename, DAP command or uncertain input automatically.
The backend—not a GPUI destructor—owns effect reconciliation, eligible checkpoint
drain and service cleanup.

Show applied versus durable checkpoint state and explicit memory-only/sensitive
recovery policy from the engine. Existing source files are never silently saved;
restored drafts go through the same baseline/conflict/authority checks as the TUI.
Native clipboard/frame/capture paths must not bypass AR08 privacy.

Exercise real window/transport close, abrupt loss and user recovery against those
completed contracts. A persistence/supervisor/protocol defect returns to the
architecture owner. An always-running reconnect daemon or live TUI↔GUI transfer
remains outside this release; don't introduce it as a “small reconnect.”

## 8. Filesystem and namespace envelope

### Required now

Native Linux files in the chosen WSL distro, existing SSH/resource namespaces and
supported container operations all execute through their existing Linux owners.
The frontend does not open workspace files through Win32 or `\\wsl$` as a second
filesystem implementation. SSH config/agents and Docker context selection come
from that WSL environment, not a duplicate Windows tool configuration.

Windows necessarily reads its own installed assets and frontend preferences;
that is not support for native Windows **workspaces**. Keep host paths distinct
from ResourceLocation. Existing shared editor settings remain in WSL's
`$XDG_CONFIG_HOME/strop/config.toml`; Windows-local GUI preferences contain only
frontend settings such as fonts/window placement/distro selection.

### Mounted Windows paths

The user explicitly permits limiting Windows filesystem access. Initially:

0056 AR05 supplies the resolved mount/namespace capability policy. The GUI renders
and obeys those decisions; it does not implement a second filesystem classifier
or enforce a restriction only by hiding buttons.

- Linux-side directory browsing/reading under `/mnt/c` can be offered with an
  explicit mounted-Windows/read-only indication.
- Edit/save/rename/move/Trash and project execution on such storage are not
  advertised as equivalent to native Linux behavior. Consume the released backend
  capability/policy and its checked action refusals, not just disabled GUI buttons.
- Use the backend's resolved mount/resource facts, not a GUI `/mnt/c` prefix test;
  mounts/symlinks can expose the same storage elsewhere.
- Reading/copying into a Linux workspace is a useful explicit route. Native Windows
  drive/UNC import, write fidelity, watchers, permissions, case behavior and atomic
  publication need the later storage milestone.

Microsoft documents different DrvFS permission/metadata behavior and recommends
Linux storage for Linux tool workloads. Read-only editor policy is not an OS sandbox:
explicitly executed programs can have broader filesystem access; do not imply otherwise.

A Windows file-dialog/drop path is not automatically a Linux path. Prefer the
backend's directory picker. Recognized WSL locations may select their exact distro
and Linux path through a checked conversion; C:\ and arbitrary UNC paths receive
an explicit unsupported/import path, never a guessed current-distro alias.

This envelope removes the need for a native Windows filesystem, Windows LSP/DAP
or ConPTY implementation in this GUI release. It does not weaken the protected
local/remote operations already required within supported Linux namespaces.

## 9. Native input, geometry and accessible editing

### Modal keys and text entry

Use the 0055 frontend-neutral input contract before lossy TUI normalization.
Windows supplies supported key/modifier/repeat/text information; the backend routes
it to source/field/terminal/debug owners. GUI menus and accessibility actions call
the same semantic commands as the TUI.

Don't silently replace Vim bindings with Windows shortcuts: Ctrl-V remains visual
block where that is the modal meaning, Ctrl-O remains jump-back and Ctrl-R remains
redo. Clipboard/menu accelerators must be additive, explicitly documented and
context-aware. Terminal-input forwards child keys; F-keys and Ctrl-C are not global
editor actions while the child owns input.

IME preedit stays frontend-local and visual. Commit is one correctly encoded input
operation for a captured input owner/revision/selection; it is not duplicated as
physical-key text. Focus/mode/document changes cancel stale composition. Candidate
placement uses the actual caret geometry, including splits, scrolling and DPI.

Pointer hit-testing uses shaped text and a checked source-byte mapping. Byte offsets,
UTF-16 accessibility/IME positions, logical lines/cells and physical pixels are
separate domains. Double-click/word/line selection and drag/block selection must
reuse the editor's meaning, not the toolkit's independent word-boundary algorithm.

### Typography and layout

Keep source text deliberately monospace, with tested fallback/Unicode behavior.
Chrome can use native proportional text without affecting grammar coordinates.
Line numbers, indentation, matching delimiters, diagnostics/breakpoints and current
execution markers all use one source geometry model. Never paint a GUI cursor
with a different mapping from pointer selection or the accessible caret.

Handle 100/125/150/200% scaling and monitor transitions, zoom/font changes, wide/
combining/RTL text, tabs, long lines and split-pane clipping. A missing glyph is not
proof of a byte mapping bug; a misaligned caret is not fixed by arbitrary padding.
Document visual/source ordering where bidirectional text differs.

### Accessibility

GPUI/AccessKit is a mechanism, not completion. Supply stable logical identities,
roles, text runs, selections, expand/collapse state, labels and actions for every
custom surface. GPUI's text! macro derives IDs from source locations; repeated
rows need explicit stable IDs or nodes can collide/disappear. Ranking/reordering
must not change logical accessibility identity accidentally.

Expose code and terminal text, structured search/FS/debug trees, dialogs and menus
through native Windows UI Automation. Test real read/navigation/selection/actions
and focus, not merely tree existence. Offscreen access uses bounded cached/fetched
text with honest pending state; synchronous UIA callbacks must not block on a WSL
round trip. Announce terminal/debug output in a bounded usable manner.

## 10. GUI composition and visual quality

Target shape, not a screenshot of implemented UI:

```text
Native window: Strop — project · Ubuntu
┌ workspace / document navigation ─────────────────────────────────────┐
│ optional Directory │ source/editor panes          │ debug inspector  │
│ or source list     │ per-pane file identity       │ when requested   │
│                    │ real source, gutter, carets   │ stack/variables  │
├ optional tools: Terminal | Debug console | Output | Operations ───────┤
│ selected tool surface; real content and ownership, not decorative UI  │
└ mode · source/namespace · effective settings · jobs/errors · location ─┘
```

Search opens the same large workspace/card contract from 0053. Replacement is an
in-place field/intent toggle; filesystem name drafts and project edits open the
same checked review/receipt models. A sidebar is optional, not a second filesystem
or document authority. Small windows collapse auxiliary views before source text
becomes unusable; restore the user's layout and selection when views return.

Use a small shared GUI component family: fields/suggestions, command rows, source
cards, tree/list rows, progress/refusal blocks, tabs/tool headers, review/diff rows
and dialogs. Maintain strong basenames, quieter parents/metadata, aligned source
lines, clear selected/active states and restrained background layers. Avoid both
a literal enlarged terminal screenshot and a cluttered IDE clone.

Debugger source-pane tracking, terminal controlling-view geometry, search retained
state and collection source identity remain their existing contracts. Do not add
GUI-only shortcuts that bypass review/save/permission or turn a temporary pane
into the owner of a long-lived process.

## 11. Testing: extend the proven layers, don't replace them

Existing 0006 tiers still matter: differential grammar, deterministic engine/
headless behavior and thin real-PTY input/restoration coverage. None proves native
GUI hit-testing, IME, graphics, Windows focus or accessibility.

Primary-source GPUI findings at the inspected revision:

- TestAppContext/TestDispatcher support deterministic state/events with TestPlatform.
  This is valuable but uses mocked platform behavior.
- HeadlessAppContext accepts a real PlatformTextSystem and optional renderer factory,
  and exposes capture_screenshot/render_to_image. Real shaping can be tested
  without pretending a full native window was exercised.
- VisualTestAppContext documentation describes real macOS rendering/ScreenCaptureKit.
- **gpui_platform::current_headless_renderer returns Some on macOS and None on
  non-macOS in the inspected implementation.** Do not assume Windows screenshot
  support merely because a cross-platform context API exists.

### Required verification layers

| Layer | Owns / proves | Does not prove |
| --- | --- | --- |
| Engine differential/contracts | Existing Vim behavior, edits/history, services/permissions, source identity, DAP/terminal state machines and native-free replay | Pixels, native input/IME, Windows integration |
| Protocol/bridge tests | Framing, versions, epochs, action ordering, partial IO, stale deltas, disconnect/unknown outcomes and bounded data | Actual wsl.exe behavior or a rendered window |
| GPUI deterministic component tests | Focus, commands, fields/trees, composition state, model binding, layout with controlled inputs/text systems | OS key delivery, GPU presentation, UIA interoperability |
| Native Windows render tests | Real DirectWrite/GPU component rendering and owned-window captures with pinned assets/metrics | Complete backend/service/installer journeys by themselves |
| Windows+WSL end-to-end | Real frontend, real Linux engine/fixtures/services, native input, source bytes, close/recovery and process ownership | Every GPU/IME/accessibility configuration |
| Hardware/accessibility/installation lane | Actual supported Windows desktop, GPUs/DPI, IME, Narrator/NVDA/UIA and clean install/update/rollback | Replaces neither hermetic contract tests nor code review |

Cross-frontend parity compares semantic outcomes and ownership, not identical TUI
cells and GUI pixels. Drive equivalent normalized commands/events through the same
engine, compare source contents/revisions, selections, mode/history, active resources,
worksets, proposals and service outcomes. Separately verify each frontend's gesture
→ action and source → visual mapping.

Pixel goldens use pinned fonts/assets and controlled scale/backend; tolerate only
measured raster differences, not arbitrary large thresholds. Capture meaningful
selection/gutter/overlay/geometry states. Real GPU and native input checks remain
separate from software/headless visual fixtures. Never update a golden blindly to
make a mismatch disappear.

## 12. An agent-drivable native GUI, from WSL

Add a **private opt-in automation interface** and a small Rust-first Windows driver,
not browser/CDP assumptions. Playwright controls browser/Electron surfaces; GPUI
is a native application. A CLI driver can be invoked from WSL via normal Windows
interop and exposed to the coding harness without adding a permanent network service.

Automation operations should cover:

- identify a specific owned application/window/session and query capabilities;
- observe a semantic/accessibility tree with stable IDs, state and geometry;
- dispatch production input/action paths, resize/focus and controlled IME test events;
- wait for an explicit applied action/backend revision/presented frame condition;
- capture an owned window/client surface and return frame/revision/DPI metadata;
- inspect lifecycle/diagnostics and close the owned test session cleanly.

No set-buffer/set-trust/set-result backdoor in the native end-to-end driver. Its
synthetic routes use production event handlers and are labelled synthetic; native
smokes use actual Windows SendInput and UI Automation. An InvokePattern action
isn't a mouse hit-test, and simulated composition isn't proof of the installed IME.

The endpoint is disabled by default, explicit on startup, current-user restricted
(named pipe ACL/nonce/session identity), bounded and non-network. It exposes no
arbitrary code evaluation or shell command handler. Never attach to an unrelated
foreground window; validate process/window identity and refuse input when ownership
or foreground assumptions no longer hold. SendInput is subject to UIPI: don't solve
access failures by running the entire editor elevated.

### Readiness and capture

Use state/frame barriers, not sleeps or “the window is non-empty.” A capture must
identify the backend revision applied and frame actually presented. “All services
idle” is not readiness: terminals, follow buffers, LSP and running debuggees are
long-lived. Wait for the specific requested effect/state/frame with a deadline.

Implement a real Windows capture path if the pinned GPUI backend still lacks one:
owned-window Windows Graphics Capture or a tested renderer readback/fence is the
proper boundary. Generic PrintWindow can fail on GPU content or DPI geometry;
Desktop Duplication can capture unrelated windows. Do not silently fall back to a
blank/partial image and pass the test.

### Runtime feasibility evidence from this research

From WSL, a disposable Windows Forms window was launched through PowerShell interop.
A driver verified its identity, sent **real mouse and Unicode keyboard events with
SendInput**, observed `Applied: WSL to Windows SendInput`, captured the window and
closed it with exit 0. The image was visually inspected. This demonstrates the
native-driver-from-WSL route, **not GPUI, IME, GPU or accessibility completion**.

The initial legacy managed-UIA probe could enumerate the window/child handles but
not usable Edit/Invoke patterns; a provider-registration attempt also failed. The
successful native-input route did not validate those patterns. This is precisely
why custom AccessKit/UIA behavior gets its own release gate. The generic capture
also showed scaling/padding limitations; it is not our future GPU screenshot oracle.

Permanent tests/driver stay Rust-first with Windows APIs/AccessKit integration;
the disposable PowerShell probe is not a second maintained test stack.

## 13. Performance, CI and development from WSL

The developer can remain in WSL. Linux engine tests/builds run through existing
Compose gates. Build the native frontend/driver with a pinned Windows toolchain
on Windows CI or an explicit Windows build worker; retrieve versioned artifacts
and drive them from WSL. Do not require moving user workspaces into C:\ to test
Windows rendering. Scratch native build/artifact locations are not workspaces.

Keep three CI roles explicit:

1. Fast hermetic Linux/engine/protocol tests and TUI regression gates.
2. Windows compilation, component/text tests and package checks; hosted-runner
   software rendering is labelled as such.
3. A real interactive Windows+WSL/hardware lane for native GPU, input, accessibility,
   mounted-path limits and installer journeys. A service/session-0 runner or missing
   GPU/WSL cannot silently skip the lane and claim a Windows release was verified.

Measure end-to-end input→applied state→presented frame using one Windows-side clock;
backend timings are separate, not subtracted from an unsynchronized guest clock.
Initial proposed gates on the documented reference machine are p95 ≤ one 60Hz
frame for warm ordinary typing and p99 ≤ 50ms; under bounded background search/
terminal activity, p95 ≤ 33ms and no long stalls. These are **targets, not measured
claims**. Record hardware/workloads and resolve an unmet budget explicitly, not by
quietly testing only a tiny file. Preserve the existing TUI latency/memory gates.

Stress large files/collections, long lines, fast typing/paste, many results, DAP
inspection, terminal output, resizing and disconnects. Bound frame cache, text
windows, queues and process counts. Every native effect has one owner and terminal
outcome; no load-bound work or allocation proportional to all documents during paint.

## 14. Implementation sequence and acceptance

1. Verify completed 0056 AR01–AR16, 0057 VF01–VF20, 0058 WK01–WK20 and 0059/0060
   feature ledgers with their assurance extensions. Missing foundations block entry.
2. Pin GPUI/platform dependencies and implement one native Windows vertical slice
   against the released backend: open → edit/undo/save → selection/IME → capture/UIA.
   This is a private native-platform proof, not the full GUI release.
3. Integrate native transport/window ownership, readonly view cache, input/effects
   and recovery UI with the existing protocol/server; exercise actual Windows+WSL.
4. Build common native components/source surface and every parity family using the
   shared identity/action/view contracts.
5. Integrate debugger and terminal GUI surfaces over their completed WSL owners.
6. Finish native accessibility/automation, hardware/latency and 0062 distribution/
   onboarding. Complete UI01–UI18 plus every §4 parity row and failure path.
7. Run existing Compose/locked gates and native Windows/WSL/package lanes, update
   docs/help/changelog/site and remove prototype or duplicate frontend code.

Acceptance includes editing and reviewing source across local/SSH/container contexts,
renaming an open dirty file, replacing/collecting matches, stale completion cancellation,
debugger attach/step/stop, nested terminal input, DPI/IME and UIA selection, bridge
loss/recovery and close during owned filesystem/debug work. Source bytes, process
state and namespace identity accompany actual native visual evidence.

Native bridge/client updates must conform to the 0057 whole-core assurance contract
and 0058's completed worker migration. Update mutation/input/source/worker/effect claims and
their production/proof/observation mapping, not only packet schemas. Changes to
ordering, acknowledgement, delta/focus or lifecycle require calibrated correspondence.
Run `model`, `verify`, `tlaps` and `core-assurance` on the exact release candidate.
They do not prove Win32, GPU, IME or UIA behavior; actual native evidence remains
mandatory and cannot be inferred from the inherited baseline.

One GUI integration owner coordinates changes to released contracts with their
0056/0057/0058 owners. No independent editing model, write/recovery path, DAP client or terminal
supervisor. Native components split by concern; use LSP references for exported
changes and migrate callers as one clean cutover.

## 15. Explicit later milestones

The user permits a WSL-first execution/storage envelope. These are not missing
parts of the stated GUI release:

- Native Windows workspaces/processes/LSP/DAP and ConPTY, including fully writable
  DrvFS/NTFS fidelity, filesystem watchers, ACL/case/reparse-point semantics.
- Native Linux/macOS GUI releases and Windows ARM64 after their own hardware,
  input/accessibility and distribution gates; TUI targets remain supported now.
- A persistent reconnect daemon, cross-process live TUI↔GUI session handoff,
  collaborative editing or a cloud remoting service.
- Extra terminal graphics/image protocols, remote interactive terminal transports
  and debugger extensions beyond the already required 0055/0060 contracts.
- Arbitrary theme/plugin/docking frameworks or broad new GUI-only editor features.

An implementation may not put LSP, debugger, terminal, remote access, filesystem
operations, accessibility or any supported parity family into this list merely
to finish sooner. Required scope changes need explicit user approval and 0028's
impact/evidence/re-entry ledger.

## 16. Primary references and evidence anchors

- [GPUI manifest](https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml),
  [platform crate](https://github.com/zed-industries/zed/blob/main/crates/gpui_platform/Cargo.toml),
  [Windows platform](https://github.com/zed-industries/zed/tree/main/crates/gpui_windows/src)
  and [Linux platform manifest](https://github.com/zed-industries/zed/blob/main/crates/gpui_linux/Cargo.toml):
  licensing, publication and actual platform/render boundaries.
- [GPUI accessibility contract](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/_accessibility.rs):
  IDs, roles, custom text/synthetic children and accessible actions.
- [TestAppContext](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/app/test_context.rs),
  [HeadlessAppContext](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/app/headless_app_context.rs),
  [visual context](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/app/visual_test_context.rs)
  and [platform renderer factory](https://github.com/zed-industries/zed/blob/main/crates/gpui_platform/src/gpui_platform.rs):
  what existing test APIs do and do not supply on Windows.
- [Zed Windows](https://zed.dev/docs/windows) and
  [remote/WSL development](https://zed.dev/docs/remote-development): native Windows
  GPU frontend and WSL workspace precedent; not a proposal to copy its GPL code.
- [Microsoft WSL GUI apps](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps),
  [WSLg architecture](https://github.com/microsoft/wslg),
  [WSL filesystem guidance](https://learn.microsoft.com/en-us/windows/wsl/filesystems)
  and [DrvFS permissions](https://learn.microsoft.com/en-us/windows/wsl/file-permissions):
  actual display and storage boundaries, not Win32 equivalence.
- [UI Automation overview](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-uiautomationoverview)
  and [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput):
  native observation/control and integrity-level constraints.
- [0006](0006-e2e-harness.md), [0029](0029-session-tracing.md),
  [0046](0046-engine-extraction.md), [0055](0055-embedded-terminal-tui-and-gui.md)
  and [0060](0060-debugger-workflow-and-architecture.md): existing Strop contracts.
- [0056](0056-architecture-prerequisites.md): completed generic architecture,
  recovery, protocol/server and current-product release-foundation entry gate.
- [0057](0057-core-verification-and-assurance.md): separately completed whole-core
  verification, source-bound claims and the feature-extension assurance contract.
- [0058](0058-unified-native-worker.md): completed native service/deployment protocol
  and assurance migration, separate from the UI-stdio presentation bridge.
