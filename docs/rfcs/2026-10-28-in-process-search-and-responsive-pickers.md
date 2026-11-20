---
rfc: "2026-10-28-in-process-search-and-responsive-pickers"
title: "In-process search and responsive picker cancellation"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-10-28"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/172"
milestone: "v0.46.4"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0077"
---

# RFC: In-process search and responsive picker cancellation

v0.46.4 replaced the local rg process with the Rust grep crates, made picker completion/cancellation survive event backpressure, added jumplist previews and moved update download, verification and extraction into Rust. Local search later became parallel in v0.60.0; SSH still runs rg where the files live (docs/search.md).

[Tracking issue #172](https://github.com/stropdev/strop/issues/172) · [Published v0.46.4](https://github.com/stropdev/strop/releases/tag/v0.46.4) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0077-in-process-search-and-responsive-pickers.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

## Search and process ownership

Local Files enumeration stays in Rust with `ignore::WalkBuilder`. Local content
Search follows Helix's implementation and uses the `grep` crate family for
matching and UTF-8 results over the same selected paths and live buffer
snapshots. Local Search never requires `rg` on PATH. Remote SSH search keeps
remote `rg` because the source files are remote. LSP servers, terminal shells,
Docker, OpenSSH, clipboard/OS integration and user `:!` commands remain
explicit external-system boundaries.

The self-updater uses rustls HTTP, SHA-256 and archive crates for download,
verification and extraction. `curl`, checksum and tar executables are no longer
required. No OpenSSL dependency is permitted.

## Liveness and ownership

Picker terminal facts are a distinct admission class from data batches. A full
semantic event lane may reject a batch with visible accounting, but it must
always admit `Finished` without waiting. In particular, picker cancellation can
never call a blocking send on the editor thread. Existing ticket equality and
permit disposal preserve generation and credit invariants.

Files and previews make scan and batch progress observable. A shared watchdog cancels a
request that produces no progress for a conservative interval; scan probes and each
searched file heartbeat, while delivery backpressure is not treated as a filesystem
stall. Remote admission and execution retain their worker/network timeout ownership
instead of being misclassified as local filesystem stalls. The failed request settles
once and refuses replacement work only while its
blocked physical owner remains outstanding. Physical completion restores availability
without creating more blocked threads. Preview selection cancels superseded reads and
local reads observe cancellation. Cache eviction preserves the selected entry where
practical.

Jump rows resolve to the already-open document through the same
`PreviewSource::Buffer` path as file/symbol previews, including the destination
line. They need no new worker, cache, or lifecycle.

Buffer-search leading `case:sensitive`, `case:ignore`, `case:insensitive`, `\c`
and `\C` use the same qualifier roles as picker qualifiers. Mid-pattern `case:`
and all other Vim regex text keep their literal styling.

## Verification

Regression coverage includes a full semantic lane during picker cancellation,
blocked Files/preview progress, physical-completion recovery, selection-change
cancellation, jumplist preview
and focus, local in-process grep parity and cancellation, updater digest/archive
success and refusal, and case-role rendering. Real PTY/headless journeys cover
Files, Search, previews and update behavior. The full Docker gate and release
assurance inventory run before tagging. No additional TLA+/TLAPS/Verus model is
required for these localized ownership fixes: the new evidence is deterministic
concurrency and behavior tests, not a second specification of the same state
machine.
