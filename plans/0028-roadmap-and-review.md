# 0028 — Editor review and prioritized roadmap

Status: P1/P2 implemented for 0.15.0 under 0031. The findings below preserve the
0.14 review baseline; they are not a list of still-open P1/P2 bugs.
Priority is impact, not module size. P0 = demonstrated data loss/security emergency;
P1 = correctness/privacy/daily editing; P2 = structural/performance quality;
P3 = optional capability or polish. No unresolved P0 was demonstrated in this round.

## Assessment

The architecture has useful foundations: a pure grammar resolver, generational
arena IDs, real buffers for surfaces, one app-event channel, revision-keyed syntax
caches and a transaction gateway. The main weakness is **partial adoption** of
those contracts, not lack of architectural documents. A typed ID cannot make a
stringly command line re-run its resolver; a correct cell grid cannot stop a raw
CR from moving the physical terminal cursor; a gateway cannot protect callers
that bypass it. Plans and comments sometimes describe a stronger system than
source implements. That mismatch should be treated as a correctness defect.

The ownership and transition work is implemented in 0031. The next separate pass
is UI polish, particularly the modeline and commit surfaces; optional capability
expansion remains P3.

## Current review — 0.18.0

The practical weakness was still work placement: asynchronous filesystem reads
did not prevent synchronous ranking, parser work, long-line prefix walks or Git
tree reconstruction from blocking rendering. 0038 now records the owner for each
path and real editor measurements; 0039 adds static syntax coverage without moving
parsers back onto input. Keep measuring the complete path, not just the matcher.

Current delivery and remaining priorities:

- **P1 delivered for 0.19 — remote editing/saving (RW4).** 0040 implements explicit
  writable admission, content-aware conflicts, protected atomic replacement,
  metadata/symlink policy, cancellation outcomes and qualified cooperative exclusion.
- **P2 delivered — WORD text-object fidelity.** `ciW`/`diW`/`daW` parse and
  resolve through the shared grammar; the same change brought word objects to
  full nvim-faithful semantics (punct runs, blank-run objects, around-trailing/
  leading blank rules, indent preservation), arbitrated by new differential
  cases. One reviewed cursor-only divergence on a REFUSED command is recorded
  in `KNOWN_DIVERGENCES`.
- **P2 delivered — chunked large forensic results.** strop-trace schema 3
  chunks oversize capture values with digest/ordering/completeness checks;
  replay assembles, metadata export stays payload-free, old traces decode
  unchanged. Delivered alongside the 0.19.1 hardening round.
- **P2 — remote-save lock release on clean exit.** `.strop-lock-<hash>` files
  outlive `:q` (0.19.1 field report). The stable inode is deliberate today;
  safe release needs the acquire-side identity recheck and a RemoteSave.tla
  amendment. Design recorded in [0041](0041-handoff-adoption-and-roadmap.md) §4.
- **P3 — trace-queue load flake watch.** One hosted-CI failure: a loaded runner
  starved the 64-deep capture writer queue, marked the trace incomplete, and
  failed `remote_edit_save_refresh...` (which replays the trace). Honest and
  loud by design; rerun passed. If it recurs, the SSH replay tests should
  tolerate an incomplete trace capture (skip the replay half) rather than
  fail the functional save half.
- **P2 delivered for 0.29 — typed input ownership and shared fields.**
  Dispatch and modeline presentation use the effective input owner; query
  suggestions, literal replacement/address fields and late cards retain their
  own acceptance/dismissal rules. The remaining engine API/render-admission
  tightening stays under 0046 rather than being claimed by this release.
- **P3 — `../` row attributes in remote listings.** The parent row renders
  `d?????????` because it is never stat'ed. One SFTP stat in the list job;
  cosmetic, batch with the next listing change.
- **Architecture handoff (S1–S10, R1, G1/G2).** Adopt/defer disposition and
  ROI ranking live in [0041](0041-handoff-adoption-and-roadmap.md), pending
  the joint review session. Next adopted slice proposal: shared resource
  identity (handoff S1).
- **Requested filesystem milestone.** Local/remote Directory buffers, file
  creation, rename/move and reviewed modal filename editing now have the explicit
  [0054](0054-unified-filesystem-workspace.md) delivery contract below; they are
  no longer an unnamed optional-directory-operations bullet.
- **P3 — optional surfaces.** GUI, Dev Containers, additional transports and
  arbitrary remote shell/debugger work retain their separate safety/platform
  prerequisites below.

Production source modules remain below the approximate 800-line ceiling; the
972-line keymap is the intentional single-pattern command listing. Keep domain
types at ownership/coordinate/protocol boundaries; private iteration indices do
not need ceremonial wrappers. Do not treat a green model as an unbounded proof.

## Current program — the multibuffer milestone (0049)

Landed from [0049](0049-product-and-architecture-handoff.md):

- 0048 input preservation (0.23.0), 0047 navigation surfaces (0.23.0).
- §4 external-header LSP continuity (0.24.0): server-originated jumps carry
  their language-service context as a routing hint consumed by didOpen;
  extensionless and ambiguous C/C++ headers inherit the navigation language;
  binding-first resolution with root-scan fallback; manual opens without
  context get a truthful route instead of install advice.

Landed in 0.25.0 (0049 §§5–8 core): the collection correctness contract
(identity, scoped undo/redo with depth preflight, immediate projection
invalidation with caret preservation, save/close semantics, g<Space>
navigation, prefix/suffix diff fast path), occurrence selection in
ordinary buffers and collections, and reviewable rename/code-action
proposals. 0.29.0 completes the 0051 live source/view contract: journal-derived
source and projection edits publish while typing, and structural source refreshes
retain source-owned caret/history positions instead of whole-view byte offsets.

Remaining work from 0049, in its original order. The 0051 release contract
below supersedes prior deferral status for its explicit R01–R11 requirements:

1. **Collection presentation (0049 §6), delivered through 0.29.0** — real
   source syntax, context, source coordinates, live split views, grouped history,
   explicit save/close and deliberate source returns are covered by 0051 R05.
2. **Reviewable project changes (0049 §8)** — rename/code-action review landed
   in 0.25.0; project replacement, exact shared diffs, consistent Apply/Save and
   per-file persistence receipts land in 0.29.0. `workspace/applyEdit` response
   timing and the broader remaining delivery work retain their separate scope.
3. **Accompanying architecture (0049 §9)** — engine public-surface
   tightening and render-side job-admission removal (0046 stage C), arena
   exhaustion checks, pinned base images and required-mode container gate.
   Collection/LSP/render/test responsibility splits and explain provenance are
   implemented under 0051; they are not remaining work.
4. **Later (0049 §10)** — structural selections before recipe syntax;
   saved working sets/named investigations; tasks with source-bound
   evidence; selective checkpoints; Dev Container lifecycle, daemon and
   GUI keep their existing evidence gates.

## 0.29.0 — whole-editor finish (0051)

[0051 — whole-editor polish and one query language](0051-whole-editor-polish-and-query-language.md)
is implemented for 0.29.0. Its §12 ledger records implementation paths, actual
surface coverage, regression evidence, measured work and explicit limitations.

**R01–R11 are all release requirements**, including uniform file/grep/replace
filters, parser-driven highlighting and query suggestions, hidden-file controls,
replacement review/apply/save consistency, complete source-backed collections,
the entire UI coverage matrix, deliberate jump/view restoration, indentation
inference and explicit controls, matching delimiters, readable module boundaries,
and integrated evidence. Severity labels do not grant permission to defer them.

The release integration owner maintains a checked acceptance ledger per R ID:
assigned owner, implementation paths, behavior, failure/cancel semantics,
exercised checks/captures, limitations and status. Reusing an already-correct
surface is fine; skipping its inspection/evidence is not.

An R requirement may move out of the release **only with explicit user approval**,
recorded here with reason, evidence, user impact, re-entry condition and linked
target plan. A roadmap bullet alone does not authorize scope reduction. Do not
call an incomplete release “v1/core/foundation complete.”

### Authorized deferral ledger

The integration owner owns these entries until a concrete implementation owner
is assigned. They are not unimplemented parts of the 0051 release contract.

| ID | Deferred scope / reason | Re-entry condition and destination |
| --- | --- | --- |
| D01 | Code completion remains unimplemented and deferred through architecture, core verification and native-worker unification | [0059](0059-nonblocking-code-completion.md) C01–C09 starts after 0056 AR01–AR16, 0057 VF01–VF20 and [0058](0058-unified-native-worker.md) WK01–WK20. Architecture/verification are already dispatched; the worker follows without interrupting them. Query suggestions remain part of 0051. |
| D02 | Full Boolean/GitHub-style query language, semantic symbol predicates, arbitrary provider qualifiers: unnecessary for the bounded uniform filters | A concrete unmet workflow after 0051's grammar, highlighting and filter-parity corpus are stable; amend/write the query plan before implementation. Do not advertise these forms meanwhile. |
| D03 | Clickable modeline/general mouse interaction: needs coherent terminal capture and hit-region ownership | A tested pointer contract that does not swallow unrelated mouse input. Keyboard `:tab-size`, its selector and visible effective setting ship in 0051 regardless. |
| D04 | Theme/typography and additional provisioning remain gated; GUI is the final parity arc after architecture, core verification, native worker, completion and debugger | 0056 closes shared gaps, 0057 verifies the baseline and 0058 unifies/requalifies native services before [0061](0061-gui-windows-and-wsl.md) consumes them for Windows presentation over WSL. |
| D05 | Completion extensions: rich snippets, additional providers, heterogeneous semantic multicursor completion and commit-character/prediction behavior | After 0059 C01–C09, under its §12 ledger and a supported interaction/ownership contract. Do not silently approximate unsupported edits. |
| D06 | Terminal TUI scope remains independent of its final GUI surface | [0055](0055-embedded-terminal-tui-and-gui.md) owns T01–T10 and preflight. 0058 preserves/requalifies its native worker/PTY integration; G01–G07 integrate under [0061](0061-gui-windows-and-wsl.md), with Windows presentation and WSL execution. |

Earlier plans for named investigations, structural recipes, Dev Container
lifecycle, installed remote services and GUI retain their own gates. They must
not displace the required improvements to today's editor.

## Requested follow-ons — search and filesystem workspaces (0053/0054)

These handoffs were requested while another session implements 0051. They are
design contracts, not evidence that that session finished or authorization to
remove its R01–R11 obligations. The integration owner schedules them explicitly;
completion in 0059 retains C01–C09 and waits for architecture, core verification and the worker.

### One Search workspace — 0053
Implemented for 0.30.0 as a separate release after 0.29.0. All S01–S10 are covered
by [0053 §11](0053-unified-search-workspace.md#11-implementation-and-acceptance--0300):
retained scoped investigations, one responsive card and included workset, and
owned exact Review/Apply/Save. Local compose, real-container, replay and terminal
evidence accompany the release; 0054 and 0059 retain their own scope.


[0053](0053-unified-search-workspace.md) requires **S01–S10**: one large stable
Search card, optional replacement toggled in place with Ctrl-R, retained
query/draft/result/workset/view state, common source rows with deliberate file
identity/line information/backgrounds, and explicit review rather than direct
replacement from the search field. Space-/ and Space-R are entry intents into
the same model, not separate catalogs. Collection promotion uses the same visible
included workset in either mode.

Keep 0051's parser, suggestions, hidden/ignored controls, source authority,
preview/review/Apply/Save semantics, whole-editor coverage and owned-worker rules.
Remove obsolete separate-kind/render/apply paths instead of keeping compatibility
branches. The S ledger requires actual surface and state-transition evidence.

### One filesystem workspace — 0054

**0.31.0 implementation complete:** F01–F12 and source-preview gutters passed
the integrated compose, required SSH/container, existing model/proof and static
release gates. Actual local/SSH TUI evidence includes modal filename copy,
dirty relocation, checked recovery, cancellation and lost acknowledgements.
0054 §15 records the exercised boundaries and the Linux/WSL coverage limitation.

[0054](0054-unified-filesystem-workspace.md) requires **F01–F12**: a common local/
SSH Directory buffer and read-only container adapter, coherent `:e`/`:browse`/
CLI/current-file reveal and path completion, useful file/line/metadata presentation,
exclusive create/mkdir, no-clobber rename/move, regular-file copy, honest Trash/
permanent removal, modal filename drafts and reviewed operations, local/SSH
Search-here routing, and live-document relocation without losing dirty text.

The original Oil/Dired filename-editing promise remains required. A read-only
tree plus dialogs is not the completed filesystem milestone. Modal drafts and
explicit actions consume one checked planner/receipt owner. Supported SSH hosts
must exercise real operations; always returning Unsupported is not capability
gating. Containers, SFTP-only hosts and unsafe operations retain named refusals.

The filesystem integration owner owns the F ledger, operation authority and
relocation boundary until concrete owners are assigned. Its evidence includes
dirty/open descendants, no-clobber conflicts, saves racing relocation, lost
acknowledgements, partial batches and mutation receipts arriving after the browser
closed. UI freshness must not erase an already-committed filesystem outcome.

### Extension boundaries, not silent scope reductions

0054 §12 records bounded follow-ons: recursive copy/permanent recursive deletion,
cross-namespace transfers, cyclic rename staging, link/metadata editing, active
workspace-root relocation, semantic LSP file-rename edits, remote Trash/bulk
replacement and durable named operation sessions. These are explicit design
boundaries, **not user-approved removal of an F requirement**. Moving required
work out needs the user's approval, impact/re-entry evidence and a ledger update.

The user authorized 0054 F01–F12 followed by [0055](0055-embedded-terminal-tui-and-gui.md)
T01–T10 as the next releases after 0.30.0, including preview line numbers and
focused search-line triangles. **All GUI work is deferred until the desired
functionality and bug-hardening are complete.** G01–G07 remain a later shared-core
frontend contract, not work to execute alongside the TUI or maintain independently.
Each release requires green CI, actual-surface evidence and verified website updates.

## Final release sequence — verified baseline, native worker, then features

The user has already dispatched **0056 architecture and 0057 whole-core verification**.
The subsequently authorized native worker is a **separate release after both**, not
a reason to stop or retarget the current proof effort. Completion remains unimplemented.

```text
0054 filesystem -> 0055 TUI terminal -> 0056 architecture -> 0057 core verification
    -> 0058 unified native worker + assurance migration
    -> 0059 completion -> 0060 debugger -> 0061 GUI (+ 0062 distribution)
```

[0062](0062-distribution-and-wsl-onboarding.md) supports GUI packaging/onboarding
and channels; it is not another editor feature arc afterward. No earlier behavior,
platform or evidence ledger is silently removed by inserting the worker.

- **Architecture — [0056](0056-architecture-prerequisites.md), AR01–AR16.**
  Finish the dispatched readonly render/action/view cutover, recovery, bindings/
  outcomes, bounded liveness, real namespace/container supervision, effects/privacy,
  UI server/driver, install/catalog/ID/explain/build contracts and current gates.
  Reuse preceding filesystem/terminal implementation. Worker implementation and
  worker-specific new proofs are not added to this agent's current scope.
- **Core verification — [0057](0057-core-verification-and-assurance.md), VF01–VF20.**
  Qualify the actual pre-worker candidate end to end, including its deployed Python
  helpers, editor mutation, local/remote filesystems, container/process/terminal/
  recovery/UI/privacy/install effects. Deliver the generalized proofs, same-source
  kernels, calibrated model/Loom/correspondence/native evidence and exact baseline.
  Do not skip Python claims because their replacement is planned. §14 explicitly
  tells an already-running verification agent how to hand off without retargeting.
- **Native worker — [0058](0058-unified-native-worker.md), WK01–WK20.**
  After the baseline, make local, SSH and supported container I/O/execution consume
  one native worker and versioned protocol. Ship a worker mode of the release binary,
  deploy it through authorized private artifact/cache flows, reuse shared Rust
  filesystem/save/supervision and remove Strop's Python helpers—including the
  supervisor, not just filesystem scripts. Preserve editor/terminal/capability
  behavior, SFTP-only reads and restricted-host refusals; no daemon or elevation.
  WK16–WK20 map every inherited VF claim, reprove changed/new kernels/theorems,
  qualify real codec/native/editor bridges and require full exact-candidate gates.
  User preference for native deployment is explicit; a “10x” speedup is not claimed.
- **Completion — [0059](0059-nonblocking-code-completion.md), C01–C09.**
  Starts only after AR/VF/WK close. Add the required two sources, indexing/query/
  resolve/acceptance, edits/undo, menu/docs, source/multicursor ownership, settings,
  privacy and real typing/server evidence over the requalified worker/service core.
  0051 static query suggestions are not this deferred feature.
- **Debugger — [0060](0060-debugger-workflow-and-architecture.md), DBG01–DBG16.**
  After completion, implement actual LLVM/debugpy DAP journeys, launch/build/attach,
  breakpoints/exceptions, inspection/console/stepping, terminals and sessions using
  the completed worker/context/lease/deployment contract. No new Python supervisor.
  `Space D` remains debugger; `Space d` remains diagnostics.
- **GUI — [0061](0061-gui-windows-and-wsl.md), UI01–UI18.**
  Last editor feature arc: native Windows GPUI rendering/input/IME/UIA, native
  automation/capture, bridge/cache integration and full supported TUI parity.
  The WSL engine owns worker-backed services. No second editor/recovery store,
  generic repair or competing worker/presentation protocol inside a GUI component.
- **Distribution — [0062](0062-distribution-and-wsl-onboarding.md), PKG01–PKG14.**
  Extend the qualified TUI/worker artifact and install identities with signed Windows
  installation, selected-WSL bootstrap, paired activation/rollback/uninstall and
  truthful public presentation. Package the exact worker-capable backend rather
  than implementing another service installer.

AR01–AR16 precedes VF01–VF20; both precede WK01–WK20; all three precede C01–C09.
Completion precedes debugger, and both precede GUI. A source defect discovered in
the worker cutover is fixed and requalified, not hidden behind a local/remote bypass
or silently deferred. All C/DBG/UI/PKG behavior and native evidence remain required.

**Verification migration is release work.** Preserve the pre-worker baseline and
its exact artifacts as history. 0058's live ledger maps claims to unchanged with
dependency justification, transferred/requalified, strengthened/reproved, new or
retired implementation-specific claims with named successors. Retiring Python does
not retire user-facing guarantees or confer proof status on a Rust port.

The worker candidate must run complete `test`, `model`, `verify`, `tlaps`,
`core-assurance` and required native/deployment/artifact lanes. New targets include
the real local protocol path, SSH/container bootstrap, verified-object activation,
lease-safe cache cleanup, child/control-stream isolation, native supervision and
worker→editor reconciliation. Keep the existing edit/composition/UI/recovery proofs.
TLC bounds, tested refinement and trusted OS/SSH/Docker assumptions remain explicit.
Publication never inherits an old 0057 badge for a new worker executable.

Every integration owner records source/contract/claim/artifact identities, failure/
privacy/cancel semantics, raw evidence and status. Required scope reductions need
explicit user approval and impact/re-entry evidence. Native GUI hardware/IME/UIA and
Windows signing/installer acceptance remain 0061/0062 obligations, not inferred from
headless core/worker proofs. The worker has no unsupported “fallback” write mode.

## Closed in 0.14.1

| Finding | Fix / evidence |
|---|---|
| #13 search deletion, backward prompt, spurious overlays | Shared resolver for live/committed searches and operator previews; shared line-edit caret mechanics; search regression suite. |
| #14 lingering physical glyphs | CRLF boundary handling plus printable-cell emission for text and metadata; real Crossterm bytes checked in a VT emulator, plus actual TUI smoke. |
| Narrow popup clamps and cursor geometry | Bounds-safe sizing; tiny resize/restore regression. |
| Predictable save temp path / private data exposure | Exclusive private same-directory staging, modes before write, no-clobber save-as. |
| Predictable updater staging | Private temporary directory and exclusive staged executable. |
| Delayed pipe slices stale UTF-8 offsets | Boundaries verified before slicing; stale results refused. |
| Shell and ranged Ex delete bypass transaction side effects | Both use the editor gateway / tx_commit. |
| Logs miss TUI handlers, truncate files or hide failures | Shared asynchronous `strop-trace`, actual LSP wire observation, common handlers, visible capture failure. See 0029. |
| Oversized mixed-responsibility modules | Client split; keymap query engine separated from listing; render tests and terminal/CLI responsibilities separated. |
| Git diff copies full text on render path despite comment | Rope clone on render path, text materialization on worker. |
| Release dependency predicates can be ineffective | Explicit rejection branch, parsed macOS dependency rows, no negated-pipeline errexit assumption. |
| Interrupted edits dropped coverage / input tokens | Smart-indent module registration and `<space>` / `<c-o>` restored; shared streaming token decoder. |

## P1/P2 delivery in 0.15.0

| Item | Implementation / evidence |
|---|---|
| R1 | Response-owned LSP request/server/document/revision/encoding; reordered, epoch-zero and equal-revision document regressions. |
| R2 | Per-incarnation didOpen/didClose and reply invalidation; external-change/reopen coverage. |
| R3 | Exclusive private atomic session/trust writes; creation-mode, symlink and partial-write failure regressions. |
| R4 | Versioned native-path serialization; non-UTF-8 and existing lossy-alias restoration coverage. |
| R5 | Compiled bounded Vim regex, actual match ranges, counted search, configured-tab blocks and CRLF editing; Neovim-derived corpus and edit/undo regressions. |
| R6 | Per-pane display-cell scroll and asynchronous open/save/native work; cursor/overlay and late-result ownership regressions plus runtime/benchmark checks. |
| R7 | One pending-input owner and reducer; full-origin selections restored on cancellation. |
| R8 | Private rope/history mutation, validated whole batches and pre-edit change journals; rejection and publication oracles. |
| R9 | Ticket-owned terminal results and process cancellation; failures/stale deliveries cannot clear newer owners, and subsequent requests progress. |
| R10 | Immutable once-built index over the existing command listing; dispatch/help contracts unchanged. |
| R11 | Native-free full forensic replay with state/cell comparison; explicit caps and payload-free metadata export. |
| R12 | Blocking terminal-event delivery rather than polling sleeps in changed tests; generated transition shrinking/replay and a targeted protocol mutant gate. |
| R13 | Named severity, revisions, request IDs, coordinate domains and register shape; private loop indices remain ordinary integers. |

## Resolved P1 findings — 0.14 baseline

### R1. Correlated LSP request envelopes

Evidence: `editor/lsp.rs::handle_lsp_event` checks navigation only when
`req_revision != 0`; revision zero is a real state. `LspEvent::HoverText` carries
no initiating document/request identity, and acceptance consults the latest
`hover_request`. Two outstanding requests against equal epochs are ambiguous.
`events.rs::connect_events` also captures position encoding when forwarding is
connected, potentially before initialization negotiates a different encoding.

Decision to design: a typed request token containing server identity, document
ID/generation, buffer revision and request sequence. A reply carries its own
owner, not a lookup against the latest mutable slot. Encoding belongs to the
server context at delivery. No zero/sentinel exceptions.

Acceptance: reverse the delivery order of two requests; switch/edit/close the
origin; exercise a pristine epoch-zero buffer and a server negotiating UTF-8.
Only the matching live request may update the UI.

### R2. LSP document incarnation lifecycle

Evidence: `document/mod.rs::close_buffer` removes the document without didClose or
clearing `lsp_opened` / `lsp_sent_epochs`; reopening can skip didOpen and inherit
an epoch-zero synchronization entry for old bytes.

Acceptance: close a file, modify it externally, reopen it, and observe a new
protocol open with the new content; old diagnostics and replies are rejected.

### R3. Private, atomic session persistence and explicit errors

Evidence: `session.rs::capture` clones undo histories, including deleted text;
`save` writes JSON with `fs::write` and default umask, ignoring serialization and
I/O errors. This is independent of tracing. A private source can leave readable
undo text in a traversable state directory.

Acceptance: session files are private from creation, atomic on update, symlink
safe, and persistence failures reach the user without falsely claiming success.
Define recovery/retention deliberately; do not call a length cap redaction.

### R4. Lossless session path identity

Evidence: `BufferState.path: String` and `to_string_lossy` in capture; restore
opens that approximation. Plan 0026 explicitly deferred this, but a lossy name
can select a *different existing file*, not merely fail to restore.

Acceptance: non-UTF-8 names round-trip without opening a lossy alias. Use a
versioned path representation with native bytes where supported; old sessions
migrate or fail explicitly. This precedes a 1.0 claim of end-to-end path fidelity.

### R5. Complete text/coordinate fidelity matrix

Evidence: normal search remains literal (not Vim regex); counts before a standalone
search can be lost when `Action::EnterText` clears parser state. Block editing
still calls `LineLayout::build(..., 8)` while rendering uses configured tab size.
CRLF display is fixed, but insert/newline policy should preserve or explicitly
choose a document's ending style. `Range` comments claim typed offsets while
its fields remain `usize`; several LSP/picker boundaries still use tuple columns.

Acceptance: byte/cell/UTF-16 conversions at non-ASCII/tab boundaries are explicit;
counted `/` and `?`, block edits under nondefault tabs, LF/CRLF insertion/undo and
regex-supported/unsupported behavior have consumer-facing tests. Do not silently
advertise literal search as full Vim search syntax.

### R6. Long-line navigation and slow I/O responsiveness

Evidence: `render/buffer.rs` renders from column zero and clips at pane width;
there is no per-pane horizontal viewport. `open_document`/save and some native
git operations perform synchronous I/O in command dispatch. Absence of `await`
alone does not prove responsiveness.

Acceptance: navigating beyond the right edge keeps the cursor/text visible;
slow filesystem/job scenarios do not freeze input, with explicit loading/saving
states and unchanged error/overwrite policy. Measure on the existing bench path.

## Resolved P2 findings — 0.14 baseline

- **R7. Pending command ownership.** `pending: String`, `pending_cursor`,
  `pending_normal`, search origin and walker state can disagree. Clipboard yank
  still seeds a synthetic grammar string. Evaluate the `PendingInput` / shared
  reducer proposal in 0030. Acceptance: one owner performs every transition;
  insert, delete, abort, accept and mode switching share one update path.
- **R8. Transaction capability boundary.** Public `Buffer.rope`, mutable history
  and ad hoc system replacement remain callable. `ChangeSet` validates ranges
  against the initial buffer but lacks a complete overlapping-edit contract.
  The TLA+ model is an abstraction, not proof of these Rust call paths or crash
  recovery. Narrow capabilities and test trace-to-model conformance before
  expanding claims. Preserve actual before/after semantics for multi-edit batches.
- **R9. Worker failure/liveness.** `refresh_hunks` catches a panic without returning
  a terminal result, leaving `hunks_in_flight` armed. Picker/preview helpers also
  turn some I/O failures into empty content. Every start needs one explicit
  completion/cancellation/failure and owner identity; no empty success fallback.
- **R10. Compile the command table once.** `keymap/lookup.rs` expands rows and
  builds token vectors during lookups. Keep the single listing, but measure an
  immutable compiled trie/index against existing correctness and p99 gates.
  No second hand-maintained dispatch vocabulary.
- **R11. Tracing evolution.** The shipped extractor replays inputs, not external
  service scheduling or filesystem history. Full traces can be large; queue
  saturation is reported, never represented as a complete capture. If field
  experience warrants it, design deterministic event injection, bounded retention
  and export-time redaction separately. Do not silently drop categories.
- **R12. Hermetic test oracles.** Existing suites include sleeps, environmental
  assumptions and tests that pin messages or compare only fresh frame buffers.
  Replace each weak oracle with a behavior/transition check as its subsystem is
  changed; targeted mutation checks should show that the assertion actually bites.
- **R13. Remaining named domain types.** Prioritize diagnostic severity, server
  request/version IDs, line/cell/byte positions and register contents (text plus
  shape) at boundaries. Private loop indexes and display strings need no ceremonial
  wrappers. No giant all-at-once numeric rename.

## P3 — after correctness

Modeline and commit/diff presentation polish shipped separately in 0.15.1 (0032).

- Search-history UX and richer diagnostics presentation remain optional P3 work.
- Debugger delivery now follows [0060](0060-debugger-workflow-and-architecture.md);
  plugin/runtime expansion remains separately deferred behind correctness.
- Crate publication now derives a real dependency topological order (0034).
- The full TRAMP-style capability roadmap is [0035](0035-remote-workflow-roadmap.md).
  Its P2 slices plus read-only directory browsing, remote LSP and remote Git are
  implemented for 0.17.0 in [0036](0036-remote-workspace-execution.md).
- Dev Container provisioning complements the same workspace/transport interfaces;
  the researched later-stage plan is [0037](0037-devcontainers-and-workspace-contexts.md).

### Final GUI arc — native Windows presentation, WSL execution (0061)

[0061](0061-gui-windows-and-wsl.md) is the current GUI contract, refining 0038's
GPUI recommendation and the user's explicit WSL-only workspace preference.
Keep the engine, files, Git/LSP/DAP, SSH/container execution and terminal PTYs in
Linux. Windows owns GPUI rendering, fonts/input/IME, clipboard and UI Automation
over a bounded versioned presentation bridge.

The generic gaps found in the sweep—mutable render admission, broad engine access,
no dirty-draft backup, lifecycle/queue bounds and the protocol/server—are owned by
0056 AR01–AR16, followed immediately by 0057 VF01–VF20 whole-core assurance.
0058 then unifies native workers and requalifies affected claims before completion
and debugger. GUI consumes those contracts; it does not repair them through callbacks.
Keep the TUI first-class and independently buildable.

UI01–UI18 and the full parity matrix require actual native Windows+WSL evidence,
including IME, accessibility, GPU rendering/capture, DPI, source/byte/pixel mapping,
all service families and protected mutations. WSLg, a mocked GPUI TestPlatform
or a non-Windows headless screenshot facility does not prove those behaviors.
Native Windows workspaces/ConPTY and other GUI platforms are explicit later gates.

### Embedded terminal program — TUI first, GUI later (0055)

TUI implementation is authorized after the filesystem release:
[0055](0055-embedded-terminal-tui-and-gui.md). **T01–T10** require a real local
PTY-backed TUI terminal, full supported input routing before editor normalization,
normal-mode text navigation, bounded snapshots/queues, process/session cleanup,
private capture/replay and actual nested-application/physical-terminal evidence.
The TUI milestone does not wait for a GUI and does not import GUI dependencies.

After the GUI's own platform gate, **G01–G07** add native rendering, input/IME,
pointer ownership, mixed-DPI geometry and accessibility over the same terminal
session/emulator/process contract. Under 0061 the native Windows frontend renders
WSL-owned Linux PTYs; native Windows workspace/ConPTY support is a later backend
gate. Sharing implementation does not imply cross-process live-session transfer.

The provisional engine choice is published `alacritty_terminal`; evaluate its
application-side input-encoding cost against `libghostty-vt` in a real preflight.
`portable-pty` is the independent PTY candidate, not a complete supervisor.
Zed's terminal crates are GPL/internal/GPUI-coupled, while WezTerm's full core has
an unpublished Git/API boundary: neither is a drop-in published MIT editor widget.
Crates.io packageability, static builds, licenses and native process ownership are
selection gates alongside terminal correctness, not post-release details.

The terminal integration owner maintains separate T and G evidence ledgers.
Removing a required milestone behavior needs explicit user approval and a roadmap
update. Current 0051–0054 requirements are not displaced by this future program.

## Verification status

Source, differential, physical-terminal and actual replay/UI evidence are collected
under 0031. Docker/hosted CI and release evidence are reported with the release,
not inferred from this review. Completing the named roadmap does not claim that
all possible editor defects or service schedules have been exhausted.
