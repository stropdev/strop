---
rfc: "2026-09-05-revision-native-git-and-services"
title: "Revision-aware Git and service results"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-05"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/114"
milestone: "v0.9.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0018"
---

# RFC: Revision-aware Git and service results

v0.9.0 preserved raw diff bytes and newline state, staged changes through structured index operations, unified event delivery and scoped language config and stale-result checks. 0021 finished revision-keyed off-thread gutter diffs and request ownership for interactive LSP replies in v0.10.0.

[Tracking issue #114](https://github.com/stropdev/strop/issues/114) · [Published v0.9.0](https://github.com/stropdev/strop/releases/tag/v0.9.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0018-revision-native-git-and-services.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

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
