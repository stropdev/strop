# 0018 — Revision-native git and services (0.9)

Status: accepted (2026-09-05 review). The git model is right
(HEAD/index/worktree/live edges); the serialization isn't. Services
return payloads without enough request identity.

## Git scope

- Byte-precise content: `DiffLine` keeps newline metadata; blobs are
  typed (`Text{bytes,encoding} | Binary | Missing`), never lossy
  `String::from_utf8(...).ok()`.
- Structured index mutation (libgit2 index/blob APIs) replaces
  hand-serialized textual patches — kills the missing-final-newline,
  CRLF, and path-quoting failure classes at once.
- `Revision` grows: `Live{doc,rev} | Worktree | Index | Commit(Oid) |
  MergeBase{left,right}` — the basis for line/selection/symbol history,
  blame-parent hops, rename following, merge-base review, historical
  jumplist locations, accurate permalinks.
- Async `DiffSnapshot` (revision-keyed, precomputed signs-by-line);
  rendering queries snapshots, never computes.

## Service scope

- `ServiceEnvelope<T> { request, document, revision, generation,
  payload }` for LSP/git/search/shell replies; explicit stale policies
  (hover: latest wins; completion: exact revision; diagnostics:
  matching-or-newer server version).
- Diagnostics honor the server's document version.
- Per-workspace language-config cache (kills the process-global
  OnceLock).
- Unified `AppEvent` source; workers wake the reducer — no 500ms poll
  latency for async results.

## Non-goals

- Incremental LSP text sync stays deferred — correct full sync beats
  incorrect incremental sync.
- No feature broadening: staging UX is fine; reader-first history
  navigation is the differentiator and comes with the Revision model.
