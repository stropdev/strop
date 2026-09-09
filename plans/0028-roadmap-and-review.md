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
- **P2 — WORD text-object fidelity.** `ciW` currently reports an invalid command.
  Add the WORD-object family through the shared grammar and differential corpus;
  do not disguise it as a deferred-input bug or add a preview-only implementation.
- **P2 — chunked large forensic results.** The existing 256 KiB per-value limit
  refuses a large Git completion rather than claiming a complete replay. Design
  bounded chunking/assembly with ownership and truncation tests before advertising
  full forensic capture of arbitrarily large worker results.
- **P2 — remote-save lock release on clean exit.** `.strop-lock-<hash>` files
  outlive `:q` (0.19.1 field report). The stable inode is deliberate today;
  safe release needs the acquire-side identity recheck and a RemoteSave.tla
  amendment. Design recorded in [0041](0041-handoff-adoption-and-roadmap.md) §4.
- **P3 — `../` row attributes in remote listings.** The parent row renders
  `d?????????` because it is never stat'ed. One SFTP stat in the list job;
  cosmetic, batch with the next listing change.
- **Architecture handoff (S1–S10, R1, G1/G2).** Adopt/defer disposition and
  ROI ranking live in [0041](0041-handoff-adoption-and-roadmap.md), pending
  the joint review session. Next adopted slice proposal: shared resource
  identity (handoff S1).
- **P3 — optional surfaces.** GUI, Dev Containers, writable directory operations,
  additional transports and arbitrary remote shell/debugger work retain their
  separate plans and safety/platform prerequisites below.

Production source modules remain below the approximate 800-line ceiling; the
972-line keymap is the intentional single-pattern command listing. Keep domain
types at ownership/coordinate/protocol boundaries; private iteration indices do
not need ceremonial wrappers. Do not treat a green model as an unbounded proof.

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
- Debugger/plugin expansion stays behind the correctness work (0019/0020).
- Crate publication now derives a real dependency topological order (0034).
- The full TRAMP-style capability roadmap is [0035](0035-remote-workflow-roadmap.md).
  Its P2 slices plus read-only directory browsing, remote LSP and remote Git are
  implemented for 0.17.0 in [0036](0036-remote-workspace-execution.md).
- Dev Container provisioning complements the same workspace/transport interfaces;
  the researched later-stage plan is [0037](0037-devcontainers-and-workspace-contexts.md).

### GUI feasibility evaluation — P3 research, no implementation commitment

Verdict from [0038](0038-remote-experience-and-responsiveness.md): keep the TUI
first-class and pursue an optional native GUI later, sharing the same editor engine.
Do not replace the terminal frontend or fork grammar, documents, jobs or replay.

Preferred first prototype: GPUI + gpui_platform + AccessKit, using our own
editor surface and pinned framework versions. GPUI's custom Elements fit code-editor
layout; its Apache-2.0 crate and native platform layer are a better initial fit than
a webview. Its pre-1.0 API churn and Zed coupling remain explicit risks. Iced/egui
are comparison/fallback candidates, not parallel implementations; Slint also adds
a declarative language and a licensing decision. 0038 records sources and tradeoffs.

The user's acceptance requirement is native Windows-first, GPU-accelerated and
visually polished while retaining strop's current minimal look. WSL is not Windows
GUI evidence. The future prototype includes mixed DPI, IME, Narrator/NVDA, driver
coverage and Windows GUI → WSL workspaces. Windows process-tree ownership and
filesystem/service integration must be ported as well; see 0038 for the concrete
DirectX/GPUI rationale and current engine gaps.

The P3 gate remains real evidence: IME preedit/commit, shaping/font fallback and
Unicode, screen-reader text/selection/actions, clipboard/HiDPI, remote buffers,
large-document latency and packaging on Linux/macOS/Windows. AccessKit in a
dependency tree does not prove a custom editor is accessible. Pixels must remain
outside byte-domain grammar and terminal display-cell geometry. A failed prototype
can still produce a no-go verdict. No GUI implementation is part of 0038's release.

## Verification status

Source, differential, physical-terminal and actual replay/UI evidence are collected
under 0031. Docker/hosted CI and release evidence are reported with the release,
not inferred from this review. Completing the named roadmap does not claim that
all possible editor defects or service schedules have been exhausted.
