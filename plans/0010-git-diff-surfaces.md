# 0010 — Diff surfaces: hunks as first-class readonly buffers

> Status: decided + implemented (0.2.1). Replaces the M3 "delta view as raw text"
> and the `Space g p` floating hunk card.

## Problem

The M3 git memory landed three diff-ish surfaces with three different levels of
polish, none of them good:

- **Delta view** (commit browser → files → Enter) dumped raw `git show --patch`
  text into a readonly buffer: `diff --git`, `index`, `---`/`+++` noise rows,
  literal `+`/`-` prefixes in the text, no old/new line numbers, fg-only colors.
- **Hunk preview** (`Space g p`) was a floating card clamped to the viewport —
  big hunks truncated, no motions, any-key-dismisses. A widget that forgot how
  to be a text editor (0001 doctrine 4 violation, honestly).
- **Changed files** rows were `{:<48}` string-padded text with no color.

Meanwhile the renderer had grown a second, drifted copy of the per-char text
loop for inactive panes (`render_text_for_pane`) — no indent guides, no diff
colors, no search hits — and diff coloring was stringly-typed
(`text.starts_with('+')` in `render/mod.rs`).

## Decisions

1. **Structured diffs in strop-git, not text.** `Hunk.lines` is now
   `Vec<DiffLine>` where `DiffLine { origin, old_lineno, new_lineno, text }`
   carries both sides' 1-based numbers and a typed origin
   (`Context | Addition | Deletion`). The `+`/`-` prefixed form is derived only
   where a unified patch is actually needed (`stage_hunk`). The commit delta
   comes from libgit2 (`diff_tree_to_tree` + pathspec) — the `git show`
   shell-out is gone; parsing our own text was the original sin.
2. **Diff surfaces are real buffers with annotations, not widgets.**
   `Surface::Diff { title, hunks, origin }` rides the existing readonly-buffer
   machinery (motions, `/`, yank, `q`, Enter). The *buffer text* is the content
   (a file-header row, `@@` hunk-header rows, unprefixed content lines); the
   *renderer* decorates rows from the typed hunk data. `Space g p` pushes a
   single-hunk `Diff` surface instead of a card — you can move in it, and
   `Space g u`/`g s` still act on the origin buffer via `origin`.
3. **One text renderer for all panes.** `render/buffer.rs` renders any pane from
   a `PaneView { buffer, cursor, view_top, overlays }` — active panes pass live
   state with overlays, inactive panes pass saved state without. The duplicated
   inactive-pane loop is deleted; inactive panes get indent guides and diff rows
   for free, and can't drift again.
4. **Diff row anatomy** (borrowed from tuicr, tuned to the strop palette):
   `[sign][old no.][new no.][content]` — the sign column carries the origin
   color (`▎` green/red, blank for context), old/new numbers are right-aligned
   dim (absent side blank, never `0`), additions/deletions get quiet full-row
   backgrounds (`#1b2620` / `#2a1d20`), hunk headers and the file-stats header
   sit on a quiet band (`#22242e`) — structural rows read as a band, not an
   accent. No `+`/`-` glyphs: the marker column and backgrounds carry it
   (column-aligned content either way).
5. **Changed-files and log rows are decorated too.** Files: path left,
   `+N -M` right-aligned and colored. Log: graph runes and `·` separators dim,
   sha accent — the graph stays legible without becoming a rainbow.
6. **Readonly dispatch trusts the grammar.** `feed_readonly` pushes every char
   into `pending` and lets `strop_grammar::parse` accept/reject (the resolver is
   the shared source of truth — doctrine 2), special-casing only `q`, Enter,
   `v`, the leader, and `:`. The hand-maintained motion whitelist is gone.

## Pushed back on (with reasons)

- **Side-by-side mode** (tuicr's second renderer): doubles the geometry
  surface for a view mode strop can't yet switch at runtime. Unified first;
  SBS lands with the layout-tree work if it earns itself.
- **Syntax highlighting inside diff rows** (tuicr's two-virtual-file syntect
  pass): strop's highlighter is tree-sitter-over-rope; faking two virtual ropes
  per hunk is real machinery. Backgrounds + gutters already carry the scan
  signal; revisit when diff surfaces get folding (below).
- **Context folding / gap expanders** (`↓ expand (N)` rows): needs
  fetch-more-context plumbing per side. The hunk surface is small today;
  folding follows the SBS revisit.
- **Picker file-preview line numbers** (rootle's numbered preview): the preview
  lives inside a floating card whose width is already tight; numbers would
  squeeze content for little gain there. The diff surfaces — the ones you
  actually navigate — have both side's numbers.

## Files

- `crates/strop-git/src/lib.rs` — `LineOrigin`, `DiffLine`, `FileDiff`,
  `Repo::commit_file_diff`; `stage_hunk` derives the patch.
- `crates/strop-git/src/memory.rs` — `show_file_delta` deleted.
- `crates/strop/src/editor/git_memory.rs` — `Surface::Diff`, surface building,
  simplified readonly dispatch.
- `crates/strop/src/editor/git.rs` — `Space g p` pushes a surface;
  undo/stage accept an explicit buffer+hunk target.

- `crates/strop/src/render/{mod,buffer,diff}.rs` — renderer split by concern;
  `render/hunk_card.rs` deleted.
- `demos/demo.tape` — `C-w` is a prefix: the chord is `C-w w`; hunk preview
  closes with `q` now.
- `.github/workflows/demo.yml` — bot artifact commits carry the skip
  marker (a gif refresh needs no gate).

## Follow-up recorded

The demo pipeline's "unchanged render commits nothing" idempotence is
unreachable while the tape has a live LSP wait: rust-analyzer startup
jitter changes GIF frame timings every render, so each merge of an
artifacts PR spawns another. Bot commits now carry GitHub's skip marker
(no gate on a gif refresh), and true convergence needs either a
deterministic tape (stubbed server, no 14s live wait) or a
content-aware diff in `demo.yml`. Not in this release.

## Side-by-side research (0.3.2)

JetBrains' side-by-side with "linage" is the best diff UX anywhere, so
we priced it for the terminal. The blocker is doctrine, not effort:
every strop surface is a real buffer whose text mirrors its layout —
cursor, `/` search, yank, and marks all work because row N on screen is
row N in the rope. Side-by-side pairs two logical rows into one display
row, breaking that mirror; keeping it would fork the surface contract
(a "view" that forgets how to be a buffer — 0001 §4). git-delta and
tuicr made the same call: unified layout + intra-line emphasis. We took
that path: del/add run pairing (same pairing a side-by-side needs)
drives two-tier emphasis — quiet row tint, loud changed span. If a
future split view ever lands, `hunk_emphasis`'s run pairing is the
alignment engine to reuse.
