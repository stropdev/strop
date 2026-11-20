---
rfc: "2026-09-06-pathbuf-paths"
title: "Native filesystem paths end to end"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-06"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/122"
milestone: "v0.13.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0026"
---

# RFC: Native filesystem paths end to end

v0.13.0 finished the part 0021 had not: CLI operands, editor open paths and language detection preserve non-UTF-8 filenames through highlight and save instead of reopening a lossy display string ([1367cb3](https://github.com/stropdev/strop/commit/1367cb32a9dcf48604cb585f59d0ed11dc884a6b), [741796f](https://github.com/stropdev/strop/commit/741796f200d46200a7f5ecd78a23a09146edd2a9)). Later session work preserved the same distinction across restarts.

[Tracking issue #122](https://github.com/stropdev/strop/issues/122) · [Published v0.13.0](https://github.com/stropdev/strop/releases/tag/v0.13.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0026-pathbuf-paths.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: landed (0.13.0). Closes the last 0020 1.0 line items:
`DocumentSource` shipped in 0021; this is the PathBuf half.

## 1. Why

`Buffer.path` has been `Option<PathBuf>` since 0021, but the editor's
plumbing is still `&str`: `open_document(&str)`, `save_as(&str)`,
`Highlighter::for_path(&str)`, and `path.display().to_string()`
roundtrips at the LSP/picker call sites, and `std::env::args()` (lossy)
at the argv entry point. A file named `b"\\xff.rs"` cannot be opened from
the command line, and an LSP `gd` into a non-UTF-8 path opens the wrong
file or fails. Linux filenames are bytes; the editor must not assume
UTF-8 anywhere a path crosses an API.

## 2. The cutover

- `Editor::open_document` / `open_buffer`: `&str` → `&Path`. The
  canonicalize + dedupe logic is unchanged (already Path-native).
- `Buffer::save_as(&str)` → `impl AsRef<Path>` (strop-core).
- `Highlighter::for_path(&str)` / `languages::detect` / `first_line`:
  `&Path` (strop-syntax). Extension/basename detection reads
  `Path::extension`/`file_name` (OsStr) instead of rsplitting a String.
- `main.rs`: the file operand comes from `std::env::args_os()`, never
  the lossy `args()`.
- LSP/picker call sites: pass the `PathBuf` through; no
  `display().to_string()` roundtrip on the open path.

## 3. What stays lossy (recorded, deliberate)

- **Session persistence** (session.rs): the session file is JSON —
  strings are UTF-8 by definition. Non-UTF-8 paths are written with
  `to_string_lossy` and simply won't restore such a buffer. Acceptable:
  a session nicety, not data loss; the file itself is never touched.
- **Ex commands** (`:e`, `:w path`): the command line is typed text —
  UTF-8 by construction. `Path::new(typed)` is the whole story there.
- **LSP protocol**: `file://` URIs are percent-encoded UTF-8 per spec; a
  non-UTF-8 path percent-encodes its bytes. strop-lsp already owns that
  boundary.

## 4. Acceptance

- Regression test: a file whose name is non-UTF-8 bytes
  (`OsStr::from_bytes`) opens via `open_document`, highlights by
  extension, saves, and a `gd`-style `jump_to_location` into it lands.
- Every former `display().to_string()` on an open path is gone.
- Gate green.
