# 0030 — Make broken transitions difficult to express

Status: the P1/P2 proposals were subsequently accepted and implemented under
0031 for 0.15.0. This document preserves the original design rationale;
0031 is the execution and acceptance contract.

## 1. Diagnosis

Generational IDs solve *which object*. Transaction gateways solve *how a change
commits*. Neither solves *which input surface owns this key*, *which query a
response answers*, or *what bytes the physical terminal interprets*. Those are
different invariants. More wrappers around IDs would not have prevented #13.

This round found concrete seams:

- `pending` is both a command/search field and a synthetic clipboard-yank grammar
  string. Its caret, mode and search origin are separate fields. A caller can
  change one without triggering all derived work.
- Search/find/preview logic formerly inferred state from arbitrary characters in
  that string. A pattern ending in `t` could activate the find overlay.
- The transaction gateway existed, but shell and ranged Ex deletion still
  committed history directly. Those callers are fixed now; the capability to
  bypass the gateway still exists.
- TestBackend represented the expected cells, but did not execute raw CR/ESC as
  terminal control bytes. A buffer-model assertion was the wrong oracle for #14.
- Protocol freshness uses epochs where a real request identity is needed; zero is
  sometimes treated as absence. A model-checked abstract request envelope does
  not prove the Rust implementation carries that envelope.

The strategy is **one owner per transition, proof at the consumer boundary, and
failure traces that shrink to a useful counterexample**. Not a rewrite and not
an effort to make every integer in the repository a newtype.

## 2. Ownership proposal

### A. One pending-input state, one edit reducer (P1)

Reuse `strop_picker::LineEdit` rather than create another line editor. Introduce
an owned pending-input enum separating structural grammar composition from text:

- no pending input;
- command line (Ex/pipe, own line editor);
- search session (direction, fixed origin selection/viewport, count/register,
  previous committed query and line editor);
- structural operator composition remains in the existing `Walker`.

The sigil is presentation derived from the variant, not mutable text that code
rediscovers with `contains('/')`. Route clipboard-yank into the walker's typed
operator/register state, not a fabricated string.

A single reducer consumes input and returns typed outcomes: edited, caret moved,
accepted, cancelled, unchanged. Every edited outcome recomputes the query once;
there is no separate caller-maintained Backspace path. Preview and execute both
consume a command from the same resolver. Enter does not restart a search from
an already-previewed cursor. Origin includes the full relevant selection state,
not just a primary byte offset if multicursor behavior is supported.

Acceptance matrix: `/`, `?`, Ex, pipe, picker query, replacement field × insert,
Backspace, normal-mode edit, caret movement, empty content, accept, cancel. Assert
observable text/query/cursor/selection transitions, not which helper was called.
Every intended deviation from Vim is named; compare shared grammar against nvim.

### B. Capability-shaped mutation API (P1/P2)

Make ordinary text reads available without exposing writable rope/history fields.
An edit transaction owns a document and its base revision. A separate,
restricted system-content update capability serves generated buffers. Commit
emits one change description consumed by history, anchors, syntax, service clocks
and diagnostics; it is impossible to commit history while skipping other owners.

Define batch overlap/order semantics before changing `ChangeSet`: replacements
at the same position, adjacent changes, conflicting overlaps and UTF-8 boundaries.
Do not implement rollback by catching a panic and declaring success. Distinguish
in-memory atomicity, persistence atomicity and crash recovery in claims/tests.

Acceptance: normal operators, insert sessions, Ex, shell, replacement, git discard
and future LSP edits all pass the same visible anchor/history contracts. A failed
batch leaves text, selections, history and revision unchanged. One registered
counterexample for each previously bypassing caller remains as a regression.

### C. Response identity and completion ownership (P1)

Use typed `RequestId`, `ServerId`, `DocumentId`, `BufferRevision` and protocol
`DocumentVersion`; do not equate their integer values. A response carries the
original request envelope. The receiving owner decides accepted/rejected exactly
once and logs that decision with its cause. Every job start ends in completion,
cancellation or failure; an exception cannot leave an in-flight flag armed.

Acceptance: deterministic permutations of replies, cancellations, closes/reopens,
server initialization and document edits. Include epoch zero and two documents
with equal epochs. Verify liveness (a failed request allows a subsequent request),
not just safety (the old result was ignored).

### D. Printable terminal data boundary (P1)

Keep the newly landed control-glyph and final-frame boundary. Extend the contract
matrix to wide/combining graphemes, tabs, CRLF/bare CR, long clipped lines, splits,
overlay open/close and resize. Source bytes remain editable and saveable; terminal
encoding is a view, not document normalization.

A cell-grid oracle tests layout/style. A thin VT-emulated Crossterm oracle tests
emission. A real PTY smoke tests terminal lifecycle/input transport. None of those
is a substitute for the other two. Do not grow a timing-heavy PTY product suite.

## 3. Testing strategy

### Stateful property tests, not random noise (P1)

Use a small state-machine generator with actions carrying preconditions. Generate
meaningful command-line edits and lifecycle transitions, plus a deliberately
separate malformed-input distribution. Bias toward boundaries: first/last byte,
empty pattern, line endings, multibyte text, zero/narrow sizes, equal revisions.

After each transition assert invariant *and observation*: valid cursor boundaries,
preview/execute agreement, query changes reflected on screen, ownership of async
results, no unauthorized persistence. Use shrinkers that retain the action's
structure, not merely random character deletion. Persist failing seeds and the
minimal script; a thousand unrelated no-ops is not useful coverage.

[Proptest's documented model](https://proptest-rs.github.io/proptest/intro.html)
combines generation and shrinking and complements example regressions. Start
with one subsystem and a bounded CI case count; do not add a project-wide fuzz
matrix before proving it finds a real old defect.

### Differential tests at the right checkpoints (P1)

Extend the existing nvim harness to checkpoints during composition and cancellation,
not just the final buffer after Enter. Pin nvim version/options and preserve the
intentional-divergence list with reasons. Compare cursor, register shape and
selected ranges as well as final text.

Neovim's [test development guide](https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/dev_test.txt)
uses fresh subprocesses, RPC-driven functional tests and explicit screen
expectations, with test identities in logs. Borrow isolation and correspondence
between test/action/log identity; do not adopt another language/toolchain here.
The existing Rust headless and AppEvent seams already provide the right base.

### Mutation adequacy, targeted (P2)

For each critical contract, demonstrate that removing the relevant guard or
skipping recomputation makes a test fail. Begin with search edit refresh, generation
checks, empty-state cleanup, UTF-8 range validation and printable cells.

[cargo-mutants](https://mutants.rs/) tests whether inserted defects escape the
suite. Run targeted modules, preferably nightly or manually first. Triage surviving
mutants for a missing consumer assertion, equivalent behavior, or unreachable
code. Do not impose an undifferentiated mutation-score percentage gate.

### Model refinement, not model theater (P2)

The current TLA+ spec covers document/request/transaction protocol state and says
it does not model text, liveness or full multi-server identity. Keep that scope
honest. Before extending it, define a mapping from actual Rust trace events to
spec actions; validate real traces against model invariants. Add pending-input
states only once the reducer contract is decided. Keep a known-bad mutant that
violates each new invariant, so an invariant accidentally weakened cannot pass
vacuously.

Tracing from 0029 is diagnostic evidence, not automatically deterministic replay
of the world. Its extractor replays external input; worker results and filesystem
state need a deliberately designed injection format before full replay can be
claimed. Large forensic traces and incomplete captures must remain distinguishable.

## 4. Adoption sequence and stop conditions

1. **Agree the input contract.** Walk the surface/action matrix with examples;
   settle sigil cancellation, Escape mode semantics, count/register preservation
   and multicursor search origin. No source redesign before this discussion.
2. **Pilot one reducer.** Migrate search plus one picker field, retaining a single
   implementation and deleting old paths in the same change. Stop if the design
   needs caller flags to select divergent refresh behavior.
3. **Prove the pilot.** A saved #13 counterexample, mid-composition nvim checkpoint,
   stateful shrinker and one killed refresh mutant. Stop if only final-text tests
   pass while cursor/highlight state remains unasserted.
4. **Seal mutations and request envelopes.** Migrate all callers; reject overlap
   and stale-owner cases with typed errors. Stop if raw writable fields or sentinel
   exceptions must remain as permanent compatibility paths.
5. **Broaden only from evidence.** Add VT-boundary properties and job scheduling
   permutations. Preserve the existing p99/input-render benchmark; no tracing,
   newtype or property-test claim substitutes for measured runtime behavior.

Each implementation tranche must fit within module responsibilities (roughly
400 healthy, 800 ceiling), remove obsolete shims and update docs/help together.
No telemetry server, mandatory configuration, OpenSSL, new async boundary in the
input path, or broad plugin/debugger work is part of this proposal.

## 5. Decisions for our discussion

- Search abort: restore the original viewport exactly, or only restore selections
  and let normal scroll policy place them? Both are testable; choose deliberately.
- Multi-cursor incsearch: live-preview every cursor or primary-only until commit?
  Match execution and make origin ownership explicit either way.
- System surfaces: a distinct mutation capability versus the same transaction
  object with a typed origin? Choose the smaller API that prevents bypass.
- Privacy/replay: full event-result capture as a separate forensic mode, or a
  curated reproducer export that omits secrets and requires supplied fixtures?
  Do not promise automatic redaction of arbitrary source/diagnostic text.

These questions were resolved in 0031: exact-origin cancellation, all-cursor live
search, journal-backed typed system mutations, and native-free full forensic replay
with a separate payload-free metadata export.
