# 0026 — PathBuf paths end to end (non-UTF-8 filenames)

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
