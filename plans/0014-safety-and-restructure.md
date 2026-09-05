# 0014 — Safety gaps and the centre's restructure

Status: accepted. Consolidates the 2026-09-04 external review, the
headless QA sweeps (rounds 1–2), and user reports into one execution
order. The review's verdict stands: the product architecture is strong;
the implementation centre is prototype-shaped. We pause feature surface
for safety + structural work.

## Verdicts on the review's claims (verified against source)

| Claim | Verdict | Evidence |
|---|---|---|
| LSP sends version 2 on every didChange | TRUE | strop-lsp `did_change` hardcodes `version = 2` |
| No position-encoding negotiation; byte cols sent as LSP chars | TRUE | no `positionEncoding` in initialize; `col_of` (bytes) → `Position.character` |
| `:wq` ignores save failure → data loss | TRUE | normal.rs `let _ = self.buf_mut().save(); close_buffer(true)` |
| readonly suppresses history, not mutation | TRUE | strop-core `insert`/`delete` check `!self.readonly` only around `history.record` |
| stage-hunk silently saves dirty buffer, discards result | TRUE | git.rs `let _ = self.buffers[idx].save()` before `git apply --cached` |
| Parallel index-keyed vectors (highlighters/surfaces/mru) | TRUE | editor/mod.rs; one alignment regression already shipped |
| Dynamic command chars as `u8` (f/t/i"a"/surround) | TRUE | strop-grammar types.rs |
| `\|` repurposed (vim: column motion) | TRUE | normal.rs pipe key |
| `q`/`Enter`/`Tab` repurposed on readonly surfaces | PARTIAL — pushback below | git_memory.rs |
| Tautological history test | TRUE | history.rs `assert!(!h.can_undo() \|\| h.can_undo())` |
| Ropey stays | AGREE | problems are coordinate ownership, not the rope |
| rg errors surface nowhere (QA round 2) | TRUE (ours) | GrepWorker nulls stderr; `-t rs` on an old rg = silent zero results |

### Pushback

- **`q`/`Enter`/`Tab` on readonly surfaces stay contextual.** Vim itself
  remaps keys in special buffers (quickfix Enter, help Ctrl-], netrw's
  whole alphabet). The rule we adopt instead: a surface must *look*
  focused (name in the modeline + `[RO]`), which strop's surfaces do.
  What we restore is the one violation on *real* buffers: `|` returns
  to vim's column motion; pipe moves under the leader (`Space |`).
- **"Everything is a real buffer" stays for git/help/log surfaces** —
  pickers already aren't buffers (strop-picker is a model), so the
  review's adjustment is already halfway true. We adopt its wording:
  everything *feels* like the editor; not everything is structurally one.
- **No slotmap crate.** Stable IDs, yes — but a 30-line generational
  arena in strop-core beats a dependency for it (house rule: boring).

## Execution order

### Wave 1 — safety (0.4.0, this plan's code)

1. `:wq`/`:w` failures keep the buffer open and dirty; message the error.
2. Atomic save: temp file in the same dir → flush → rename; preserve
   permissions; refuse to overwrite a file changed on disk since load
   (mtime stamp) without `:w!`.
3. Readonly enforced at the mutation boundary: `Buffer::insert/delete`
   refuse on readonly; generated surfaces use an explicit
   `replace_all_system` owner path (review §"read-only").
4. LSP: per-document monotonic versions; negotiate position encoding
   (`utf-8` preferred, `utf-16` honoured); one conversion module with
   emoji/combining/astral tests; conversions at the lsp boundary both ways.
5. Stage-hunk: refuse on dirty buffer ("`:w` first"), propagate save/apply
   errors — never write unrelated edits to the worktree silently.
6. GrepWorker streams rg stderr; non-zero exit with an error posts it to
   the picker instead of a silent empty list.
7. Session saves on clean exit (not only `:w`). [done, uncommitted]
8. `|` = column motion again; pipe = `Space |`. History tautology test
   replaced with a real assertion.

### Wave 2 — identity & coordinates (plan 0015, next)

- `DocumentId`/`ViewId`/`PaneId` generational arenas; kill the parallel
  vectors; `Document` owns text+history+syntax+stamps; `View` owns
  selections+scroll.
- Typed coordinates: real newtypes in `strop_core::id`
  (`ByteOffset`/`LineIndex`/`ByteColumn`/`Utf16Column`/`DisplayColumn`),
  conversions centralised. **Shipped shape (0.5.x):** the Buffer API takes
  `impl Into<ByteOffset>`/`impl Into<LineIndex>` — a line index passed
  where bytes go stops compiling, which is the observed bug class;
  LSP columns convert through the tested `to_server_col`/`to_byte_col`.
  Internal byte arithmetic stays `usize` (post-0.3.9 boundary-honest);
  full internal newtyping continues per call-site cluster as waves touch
  them — big-bang rewrites of every arithmetic site are unreviewable.
- `SelectionSet` unifies cursor/visual/extra-cursors.
- LSP server pool keyed by (root, server) — rust + pyright + clangd in
  one session.
- `MotionShape { Characterwise{inclusive}, Linewise, Blockwise }`;
  dynamic command chars become `char`.

### Wave 3 — input & plans (plan 0008 grown up)

- Key-event trie + typed parser state (register/count/op/count/motion).
- One command registry driving dispatch, which-key, `?`, dot-repeat
  (semantic, not string replay), macros.
- `ActionPlan` (edits + selections + registers + mode + effects) as the
  single preview/execute object — "the preview cannot lie" becomes
  structural.
- Neovim differential harness (0006 tier 3) pinned to one nvim version;
  property-generated corpora; every new motion ships with diff cases.

### Wave 4 — git as a data model

- Four explicit states: HEAD → index → worktree → live document; every
  git command names its edge; gutter distinguishes
  unsaved/unstaged/staged/committed.
- `SourceLocation { repo, revision, path, range }` —
  jumplist/blame/search/permalink over historical revisions.
- Archaeology first: line/selection/symbol history, changed-symbol
  navigation, branch review — ahead of more staging UI.
- Explicit unsupported states: binary, renames, CRLF, modes, merges,
  conflicts.

### Deferred surface (tracked on the site under Next)

ZZ · `{`/`}`/`(`/`)` motions · visual block · `:%s` and ex ranges ·
`ctrl-^` · `ge/gE` · `g;`/`g,` · `gi` · `gv` · H/M/L · zz/zt/zb ·
Ctrl-d/u/f/b · tree-sitter text objects (af/ic/aa…) · LSP
references/implementation/symbols/diagnostic-traversal · grapheme
clusters (0001 §5.9) · perf: snapshot-based diff/LSP sync, gutter
interval index · plugin boundary (process/WASM, post-registry).

## Testing bar for waves 1–2

- Every fix lands with the failing scenario as a test first.
- Async: fake clock + in-process fake LSP server; response reordering
  tests (close-doc-while-hover-in-flight, edit-twice-before-diagnostics,
  stage-while-index-changes-externally, save-failure-during-wq).
- No shared `/tmp` fixtures anywhere (0.3.9 flake class).
- Headless `wait`/`settle` stay the only timing in tests.
