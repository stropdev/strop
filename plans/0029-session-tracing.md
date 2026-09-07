# 0029 — Diagnostic session traces

Status: diagnostic capture shipped in 0.14.1; schema-2 full replay and bounded
metadata export are implemented for 0.15.0 under 0031.

## Contract

A requested trace must be useful after a crash, must never overwrite user data,
and must not perform file I/O or wait for the writer on input→render. One JSONL
sink serves the binary, core mutation layer and LSP client. The previous draft's
synchronous file mutex, swallowed errors, synthetic zero-byte LSP messages and
headless-only drain instrumentation do not meet this contract.

`strop --log` / `--log=ALL` enables all diagnostic categories. `--log=PATH`
or `--log-file PATH` selects a new file; `STROP_LOG=PATH` is the environment
alternative. Flags work with `--headless SCRIPT [FILE]` and `--script SCRIPT`.
Existing trace files are refused, not truncated. Relative paths resolve before
project-directory changes. Log creation errors fail startup visibly.

## Sink and schema

The shared `strop-trace` workspace crate owns storage. Its public surface is:

- `start(path: &Path, options: TraceOptions) -> Result<TraceSession, TraceError>`;
  `TraceOptions { content: ContentPolicy, limits: Limits }`.
- `enabled() -> bool`, `capture_content() -> bool`.
- `record<T: Serialize>(kind: EventKind, fields: &T)` and lazy
  `record_with(kind, || fields)`; disabled logging does not construct fields.
- `take_failure() -> Option<String>` reports a writer/overflow failure once to the
  event loop. `TraceSession::finish(self) -> Result<(), TraceError>` drains and
  joins the writer outside input dispatch. Drop also closes the writer.
- `EventKind` is a closed vocabulary: `SessionStart`, `SessionEnd`, `Input`,
  `Paste`, `State`, `Document`, `Mutation`, `History`, `Render`, `Resize`,
  `JobStarted`, `JobFinished`, `JobRejected`, `LspMessage`, `Replay`, `TraceEnd`,
  `Error`, `Panic`.

Each envelope contains `schema_version`, `seq`, `elapsed_us`, `event`, `fields`.
The writer owns monotonically ordered sequence numbers. A bounded queue uses
nonblocking admission; failure or overflow is visible in status, in the final
result, and where possible a terminal error record. It never silently produces
an apparently complete trace. JSON encoding is guarded by the enabled check;
only the writer thread touches the file. Flush each batch, not merely on quit.
Unix trace files are created with mode 0600 using exclusive creation.

Schema 2 reserves a terminal `TraceEnd` record. Hard limits are 64 MiB total,
100,000 records and 256 KiB per record. A cap, queue failure or writer failure
cannot masquerade as a complete capture; full replay rejects incomplete input.

Event payloads have explicit units and retain document slot AND generation
alongside a buffer-incarnation identity. Service receipts and rejection reasons
are distinct; applied outcomes appear in subsequent state/mutation records.
Actual LSP framing records direction, request id, method where present, payload
byte length and error metadata. JSON-RPC bodies are never written to the log.

## Coverage

- Session metadata and startup errors are logged before opening files or starting
  language servers. Panic reports include location and backtrace; terminal
  restoration remains owned by the TUI boundary.
- Every external key and bracketed paste is recorded. Macro-generated keys are
  distinguished so extracting external input cannot duplicate macro execution.
- Post-input state includes mode, pending text and caret, walker state,
  selections/anchors, search origin and committed pattern, document revision,
  panes/viewport, messages, picker state and quit state. Zero-document quit is a
  valid snapshot, not a crash.
- Mutation records are emitted by buffer mechanics, not only on insert-mode Esc:
  uncommitted edits, system replacements, undo and redo are visible. Byte units
  are explicit; text appears only in full-content mode. Commit markers are separate.
- TUI and headless use the same service handlers; logging belongs at those
  handlers, not solely in headless drains. Record stale results and errors with
  their identity/revision reason. Record actual LSP transmit and receive traffic.
- Render records describe the actual cell-grid output and cursor/viewport,
  including consecutive frames and frame duration. Headless uses a persistent
  terminal, so its frames exercise the same diff lifecycle.

## Privacy and reproduction

The default trace contains typed keys, search/command text, paths and diagnostic
messages. These can be sensitive: inspect before sharing. Edit, clipboard/paste
and whole-file text and rendered glyphs require explicit `--log-content` (`Full`
policy). Metadata frame records contain dimensions, cursor and a cell/style hash.
Error previews may be length-bounded; that is NOT redaction. All payloads are
JSON-escaped and files are private by default.

Full-content captures contain a closed forensic stream: the initial seed, external
actions, owned requests, service events, logical observations and an explicit end.
`--replay TRACE` reconstructs it without native file reads, shell commands, Git
operations or LSP traffic. State and terminal observations must agree at each
recorded boundary. Cell observations use row runs, retaining symbols, colors,
underline colors, modifiers and skipped-cell state.

`--export-metadata TRACE` is a positive projection containing only sequence
numbers and the closed event categories. Keys, paths, native path bytes, content,
commands, messages, errors and arbitrary nested fields do not survive. This
export is not replayable. Neither the default diagnostic policy nor length limits
are described as automatic redaction.

## Acceptance

Exercise the actual CLI to produce and parse a trace containing startup, input,
state, immediate edits, commit/history, consecutive rendered frames and clean
shutdown. Exercise failed log creation and writer failure. Prove a second init
cannot truncate the active trace, no disabled-log side effects occur, and
parallel producers yield ordered parseable JSONL. Drive a real TUI session with
tracing and inspect the terminal and saved log. Verify non-UTF-8 operands remain
Path-native. Every claimed event category must have a real producer.

Plan 0030's broader P1/P2 work was subsequently activated by 0031.

## Usage and exercised evidence

```sh
strop --log-file issue.jsonl path/to/file.rs
strop --headless steps.keys path/to/file.rs --log-file issue-full.jsonl --log-content
strop --replay issue-full.jsonl
strop --export-metadata issue-full.jsonl > issue-metadata.jsonl
strop --replay-script issue-full.jsonl > replay.keys
# Inspect replay.keys before running it: recorded commands may write files/run shells.
strop --headless replay.keys
```

`--replay-script` remains the explicitly input-only extractor, not the forensic
replay engine. It checks schema/sequence continuity, excludes macro/synthetic
expansion and preserves literal Unicode/angle-bracket keys. Executing that script
can repeat native side effects; full replay cannot.

Process-isolated CLI regressions exercise full-capture extraction, metadata paste
privacy, existing-file refusal and a quit that removes the last document. Shared
sink tests cover concurrent ordering, disabled lazy producers, exclusive private
creation, repeated lifecycle and real writer failure.

A local protocol fixture exercised 13 actual tx/rx frames (initialize, didOpen,
didChange, hover, shutdown and exit) with observed nonzero byte lengths. A real
TUI run exercised search/Backspace, edit/Esc and quit; the recorded frame matched
the edited CRLF fixture and the process exited 0. These are runtime observations,
not a claim that all possible external service schedules are replayed.
