# 0024 — The transaction gateway + verified state machine

Status: accepted 2026-09-06 (the author's call, sequencing overruled:
this is the next evolution, not "1.0 someday"). Review 4 asked for the
gateway; gripsack.dev's TLA+ practice (action guards, crash actions,
named invariants, mutation traces) tells us how to *prove* it.

## Part 1 — the gateway

One function through which every document mutation flows:

```rust
fn apply(doc: DocumentId, base: Revision, change: ChangeSet) -> Result<Committed, ApplyError>
```

It alone: validates the base revision, applies the edits, records
history, advances the text clock (epoch), emits the anchor ChangeMap,
bridges tree-sitter, marks dirty, and notifies services. Callers
produce ChangeSets; nobody touches the rope directly.

- `ChangeSet { edits: Vec<Edit>, undo_group: UndoGroup }` — undo
  grouping is separate from the text clock (every mutation advances
  the revision; an insert session stays one undo unit).
- `Committed { new_revision, map: ChangeMap }` — the map feeds every
  anchor in the workspace.
- Validation failures are typed: StaleRevision, InvalidRange,
  ReadOnly — never a panic, never a silent no-op.

Migration order: typing path first (biggest traffic), then
undo/redo, replace, git discard, shell filters, ex edits. Each step
keeps the suite green.

## Part 2 — verification, two layers

### TLA+ (specs/)

`EditorProtocol.tla`: variables = documents (set of live ids), panes
(id → doc), per-doc revision, pending service requests (id → doc),
transactions (open/committed). Actions: OpenDocument, CloseDocument,
SplitPane, ApplyChange, CommitTransaction, DeliverServiceResult,
Crash. Invariants, named after the failure they forbid:

- **NoStalePane**: every pane references a live document.
- **NoWrongDocument**: a service result applies only to its request's
  document.
- **MonotonicClock**: the text clock never moves backward.
- **AnchorsTrack**: marks/selections are within the document's extent
  after every commit.
- **NoPartialCommit**: after Crash mid-transaction, recovery either
  sees the whole change or none of it.

TLC checks every interleaving. Kept mutants (deliberately broken
variants) ship as TTrace files proving the spec catches each bug —
the 0023 probes are the mutant candidates.

### Model-based conformance (Rust)

A reference model (plain String buffer + the invariants above) driven
by generated operation streams; the real editor runs the same stream
through the production event path (the headless harness). Every step
asserts the invariants on the real editor and compares text/cursor/
mode/registers/undo against the model. This is the "differential gate
grows teeth" item, built on the gateway.

## Sequencing

After 0023 (the probe round) lands: gateway core + migration
(0.11.0), then the TLA+ spec + conformance harness (0.11.x).
The debugger and plugin boundary wait for this — a verified core is
what they attach to.

## Non-goals

- No runtime verification framework in the binary (the spec proves the
  protocol; the harness proves the implementation).
- No blanket rewrite of the rope or the UI.
- Liveness properties: progress is one crash-free run, by
  construction (gripsack's rule).
