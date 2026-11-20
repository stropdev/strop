---
rfc: "2026-09-06-documentsource-pathbuf"
title: "Typed document sources and native buffer paths"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-06"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/117"
milestone: "v0.10.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0021"
---

# RFC: Typed document sources and native buffer paths

v0.10.0 introduced DocumentSource and PathBuf on buffers, moved gutter diffs off the render path and attached ownership information to interactive LSP replies ([07d7b9d](https://github.com/stropdev/strop/commit/07d7b9df4e54530e143b17881d820e5f6793d9a0)). This did not yet make every open path byte-preserving; 0026 completed that cutover in v0.13.0.

[Tracking issue #117](https://github.com/stropdev/strop/issues/117) · [Published v0.10.0](https://github.com/stropdev/strop/releases/tag/v0.10.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0021-documentsource-pathbuf.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: accepted 2026-09-06. Two threads: finish the remaining 0018
seams honestly, and formalize the document model the surfaces have been
pretending into. The perf bench suite is explicitly out (deferred per
the author until there's something to measure against).

## Thread 1 — the remaining 0018 seams

1. **The gutter's diff runs on the render path.** `render` calls
   `refresh_hunks` synchronously; an edit materializes live text and
   diffs in-frame. Fix: the gutter's hunk set becomes a
   revision-keyed snapshot — edits bump the document's revision and
   enqueue a diff job; the job computes against an immutable rope
   clone and posts back `DiffSnapshot { revision, hunks }`; render
   only ever reads the last good snapshot (stale gutters are honest:
   they clear when the revision moves, never paint wrong signs).
2. **The service envelope, completed.** Hover and diagnostics carry
   identity now; goto/locations replies get (request id, document,
   revision at request) and are dropped when the asking document
   moved on. One `ServiceRequest { id, doc, revision }` type in
   strop-lsp; every interactive reply threads it.

## Thread 2 — DocumentSource + PathBuf

3. **`Buffer.path: Option<PathBuf>`** — Unix filenames aren't UTF-8;
   the current String path makes the filesystem model a UI model.
   Display edges use `to_string_lossy`. This is a mechanical migration
   across editor/git/lsp/picker with the gate as proof.
4. **`DocumentSource`** in strop-core:

   ```rust
   enum DocumentSource {
       File { path: PathBuf, disk_stamp: Option<SystemTime> },
       Scratch,
       /// A git-memory surface (log/files/diff/blame/undo) — its
       /// identity is the surface, and content is job-owned.
       Surface(SurfaceKind),
       CommandOutput(String), // "sh: make test"
       Help,
   }
   ```

   `readonly`, `name`, and "pathless" stop being conventions:
   they derive from the source. `Buffer::save` on a non-File source
   is a typed error with the source's name in it. The editor's
   git_memory surface bookkeeping folds into this — the parallel
   `surface: Option<Surface>` slot on Document goes away, replaced by
   the typed source carrying the surface payload.

   This is also the editor-module split it drives: `Document` moves to
   `editor/document/` with `mod.rs` (lifecycle) + `surfaces.rs`
   (SurfaceKind + construction) — the document layer stops living in
   one long file of mixed concerns.

## Non-goals

- The perf bench suite (deferred by the author — no claims without it).
- Incremental tree-sitter (next cycle's headline, with the bench suite).
- New user-visible features of any kind.

## Exit criteria

- Render never computes a diff; the gutter reads snapshots.
- Every interactive LSP reply is droppable-stale by construction.
- `rg "path: Option<String>" crates/` is empty.
- A file with non-UTF-8 bytes in its NAME opens and round-trips.
- A git surface's `save` attempt errors with the surface's name.
- Suite + docker gate green; no file over the 800 ceiling except the
  keymap listing.
