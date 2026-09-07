# 0031 — Execute the P1/P2 roadmap

Status: implemented for 0.15.0. All R1–R13 acceptance items are covered by
the implementation and verification below; P3 feature expansion remains out of scope.

## Decisions made before code

- Search cancellation restores the initiating selections and viewport exactly.
  Incsearch resolves every cursor from that saved origin, matching execution.
  Counts and registers survive entry into the search prompt.
- Text prompts own their `LineEdit` and context in `PendingInput`; structural
  operator composition stays in `Walker`. Clipboard yank enters typed operator
  state, not fabricated grammar text. One reducer reports edit/accept/cancel.
- LSP responses carry their own request, server, document incarnation, revision
  and negotiated position encoding. Zero is valid, never "not provided".
- Mutable rope/history storage is private. Ordinary edits use one validated
  transaction capability and publish a change journal. System updates have an
  explicit separate origin; neither can skip downstream synchronization.
- Batch coordinates refer to the pre-edit document. Adjacent edits are valid;
  conflicting overlaps are rejected before any mutation. A replacement is one
  atomic range replacement, not ambiguous independently sorted insert/delete pairs.
- Session files use private exclusive staging and atomic replacement. A versioned
  native-path encoding round-trips Unix bytes and Windows UTF-16. Ordinary legacy
  UTF-8 sessions migrate; ambiguous lossy legacy names fail explicitly.
- File open/save, slow native git work and session writes during editing run on
  workers. The event loop accepts owned results against request/document/revision
  stamps. A failed/stale result cannot switch focus, overwrite edited bytes or
  incorrectly mark a buffer saved. Startup loading may remain synchronous before
  the terminal event loop exists. Explicit loading/saving status is part of the UI.
- Each pane owns horizontal display-cell scroll; glyphs, overlays and all carets
  use the same clipped layout, including tabs and wide/combining graphemes.
- Regex search follows a documented Vim-compatible dialect. Unsupported constructs
  produce a typed query error, not a silent literal interpretation. Match ranges
  are explicit; highlighting never assumes match length equals pattern length.
- The command listing compiles once into an immutable index. No second dispatch
  vocabulary is introduced.
- Full forensic traces can inject recorded service results deterministically.
  Metadata exports explicitly remove content-bearing fields and are not described
  as replayable. Bounded capture must end with a visible limit/incomplete marker,
  never silently omit categories. No telemetry service or automatic secret guessing.

## Tracked acceptance: every roadmap item

| Item | Delivery | Required evidence |
|---|---|---|
| R1 | Typed LSP request envelopes and delivery-time encoding | Reordered replies, epoch zero, equal-revision documents and negotiated UTF-8 |
| R2 | didClose and per-incarnation open/version lifecycle | Close/external-change/reopen opens fresh content and rejects old replies |
| R3 | Private atomic session writes with visible errors | Permissions from creation, symlink safety, failure preserving old session |
| R4 | Lossless versioned session paths | Non-UTF-8 path and existing lossy alias never collide; legacy migration/error |
| R5 | Counted/regex search, configured-tab blocks and preserved line endings | Differential supported regex cases, explicit unsupported errors, Unicode/tab/CRLF edit+undo |
| R6 | Per-pane horizontal view and nonblocking file/native I/O | Long-line cursor/overlay alignment and delayed-job interaction smoke; benchmark |
| R7 | One pending-input owner/reducer | Surface×edit/accept/cancel matrix and full-origin multicursor restoration |
| R8 | Sealed mutation capabilities and validated batch overlap | All callers migrated, rejected batch leaves state unchanged, trace/model invariant check |
| R9 | Owned worker lifecycle with terminal results | Failure/cancellation/stale result permutations, subsequent request makes progress |
| R10 | Once-built command index | Existing dispatch/help contracts plus measured lookup/input-frame comparison |
| R11 | Deterministic event injection, bounded capture and metadata export | Service interleaving reproduction, explicit cap marker, sensitive fields absent from export |
| R12 | Hermetic behavior oracles and targeted mutation adequacy | Replace sleeps/environment dependence in changed suites; generated transitions shrink/replay; kept mutants fail |
| R13 | Named domains at boundaries | Severity, revisions/request IDs, byte/line/cell/server positions and register shape cannot be mixed |

## Integration ownership and shared interfaces

Main integrates `Editor` state, the shared app-event boundary, core mutation/coordinate
APIs, trace schema and release metadata. Source owners work in disjoint modules;
shared-file updates are serialized here, not performed concurrently.

Core exposes existing coordinate types plus `BufferRevision` in `strop_core::id`,
read-only `Buffer::text()` / snapshot access, and a journal-backed transaction API.
`DocumentId` serialization preserves both slot and generation. Domain conversions
remain explicit at LSP and terminal boundaries.

The LSP owner supplies `RequestStamp`/`ServerId`/`RequestId` and response-owned
encoding in `LspEvent`; `Editor::handle_lsp_event` consumes the event without a
separately captured encoding. Main migrates `AppEvent` and trace consumers.

The pending-input owner supplies `PendingInput` in `editor/pending`, replacing
independent pending text/caret/mode/origin fields. Main integrates the one field;
render and trace code consume accessors instead of inspecting strings.

The document I/O owner supplies `IoState` and typed result/intention types in
`editor/io`; Main installs its state/channel and routes results. LSP navigation
and pane/file commands use the same asynchronous opening path. Fixture-only
synchronous loading is not retained as an alternate production dispatch path.

Each owner skips formatters, builds, linters and test runs during concurrent edits.
Main performs focused reproduction/smokes after integration and runs the complete
Docker fmt/clippy/test/model gate on the final tree. Public API changes require
reference analysis and complete caller migration. No compatibility shims, silent
fallbacks, raw mutation escape hatches, or source files beyond the size ceiling.

## Delivery sequence

1. Establish shared domain and ownership interfaces; implement independent LSP,
   persistence, input/search, viewport/I/O and command-index changes.
2. Integrate worker and mutation journals, forensic replay/export and trace/model
   correspondence; eliminate stale paths and migrate every caller.
3. Run consumer-facing regressions, stateful/differential checks, targeted mutant
   probes and actual TUI/headless delayed-job/long-line scenarios. Measure the
   existing performance suite instead of inferring speed from types.
4. Update roadmap statuses and release notes, remove throwaway probes, pass Docker
   and hosted CI, then publish a new immutable tag through the existing workflow.

No item above is silently downgraded to a plan or deferred because it crosses
modules. If implementation reveals a genuinely incompatible product choice,
record the concrete tradeoff and use the established editor contract as default.

## Runtime evidence for 0.15.0

- Real CLI capture/replay: shell output, asynchronous file open, linewise pipe and
  undo. The input file and shell-created file were removed before replay; replay
  reproduced the final state without recreating either file.
- Metadata CLI export: 1,095 event records contained only sequence/category fields;
  no source, path, command or message payload survived.
- A 300,000-byte seed exceeded the record cap: capture exited nonzero with
  `TraceEnd.complete=false`, and full replay refused the incomplete capture.
- Actual 120×40 TUI: incremental search and Backspace, exact-origin cancellation,
  shortening a line, CRLF save, and independent split origins (0 and 149 display
  cells). The process exited cleanly with a complete capture.
- Neovim-derived Unicode range witnesses include ASCII-to-Unicode ranges, large
  Unicode ranges and negation. The pre-fix CLI missed `[a-é]` in `øé`; the corrected
  CLI lands at byte 2, matching Neovim.
- Production Rust files are below the size ceiling; the existing declarative
  keymap listing is the documented single-pattern exception.
- The captured real TUI session also replayed to zero documents / clean quit.
- Input-only extraction now takes its initial document from the forensic seed,
  before the first key; the process-level round-trip regression covers this order.
- Docker's complete fmt/Clippy/test stage and Neovim differential check passed.
  The separate Docker TLC gate passed the clean model and rejected the kept
  freshness mutant. Hosted CI and publication use the repository workflow.

## Release benchmark comparison

Same workstation, Linux musl release binaries: the official 0.14.1 binary and
the 0.15.0 Docker bench service. This is a measurement, not a portable timing
guarantee or attribution of every difference to the command index.

| Scenario | 0.14.1 | 0.15.0 |
|---|---:|---:|
| 120×40 input + frame, p50 | 1.15 ms | 0.80 ms |
| 120×40 input + frame, p95 | 1.28 ms | 1.20 ms |
| 120×40 input + frame, p99 | 1.53 ms | 1.36 ms |
| 80.2 MB initial buffer, p50 | 61.78 ms | 65.45 ms |
| 100 inserted keys in 80.2 MB buffer, p50 | 0.12 ms | 0.14 ms |
| 100k-line `/needle` gesture, p50 | 26.32 ms | 30.25 ms |
| One edit across 1,000 cursors | 0.73 ms | 1.04 ms |

The search row includes typing the complete incremental-search gesture, not
one frame. Input/frame tail latency stays below the 16.7 ms reference budget;
file-size-independent editing remains intact. The release image also passed
the static-link check.
