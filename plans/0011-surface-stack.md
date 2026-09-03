# 0011 — The surface stack: guaranteed returns, job generations, blame gutter, commit sidebars

> Status: decided (0.2.2). Extends 0010 (diff surfaces) and 0003 §4 (git surfaces
> as buffers, no git mode). User feedback milestone.

## Problem

Three structural gaps in the M3 surfaces, plus two feature asks:

1. **Return-point restore is conditional.** `close_buffer` only restores the
   origin buffer's cursor/view when that buffer *happens* to be current after
   the close. Leave a log open, switch buffers, come back, press `q` — you land
   on line 1 of some other buffer. The return point is a promise; it must hold
   unconditionally.
2. **`q` in a split closes the wrong thing.** Readonly `q` called
   `close_buffer`, which removes the buffer from the buffer list while other
   panes may still reference its index — `:q` semantics (close the *pane*,
   keep the buffer, only the last pane closes the buffer) already exist in
   `close_pane_or_buffer` and were not being used.
3. **The stale-log-job race.** `GitJob::Log` carried a bare buffer index. Close
   the log before the job lands and the index is recycled by the next buffer —
   the drain happily `replace_all`s log text into an innocent file buffer.
4. **Blame archaeology** (user ask, 0001 pillar 3.3 "toggleable blame column"):
   rootle-style left-margin blame — `sha author age` per line — with Enter
   diving into that line's commit.
5. **Commit diffs have no file context** (user ask): a file delta reached via
   log → files → Enter knows nothing about its sibling files. tuicr-style
   left sidebar with the commit's changed-file list, `]f`/`[f` to walk them.

## Decisions

### 1. The stack (naming what already exists)

A *surface* is a readonly buffer: `surfaces[i] = Some(Surface)` rides the
buffer list. **Push** (`push_surface`) appends and switches to it; only the
first surface pushed from a plain buffer carries a `ReturnPoint`
{buffer, cursor, view_top} — the chain log → files → delta unwinds one `q` at
a time, each close restoring the buffer below it in the stack. **Pop** is
`q`/`:q`: routed through `close_pane_or_buffer`, so in a split the *pane*
closes (buffer stays, vim `:q` semantics) and only the last pane's close
closes the buffer — at which point the return point restores **always**: if
the origin buffer is not current, switch to it (MRU + git re-discover), then
restore cursor and view. One invariant, no conditions.

### 2. Job generations

Buffer indices are only stable while the buffer list is. Every buffer-list
mutation (open, push, close) bumps `Editor.generation`; every index-carrying
git job captures the generation it was spawned under. The drain drops results
whose generation is stale — dead surfaces cannot be resurrected by index
reuse, and log text can never land in a file buffer. Path-keyed results
(the blame gutter) additionally require the gutter entry to still exist, so a
toggled-off gutter cannot be re-filled by a late job.

### 3. Blame gutter (`Space g b`, 0001 pillar 3.3)

`Space g b` on a file buffer toggles a per-buffer blame **gutter**:
`git blame --line-porcelain` for the whole file runs as a git job. The column
(`sha˟7 author˟9 age˟3`, 22 cells) renders left of the existing sign+number
gutter — composed in `render_pane`, never a second renderer. Muted for old
commits, accent for recent (< 30 days) and uncommitted (`0000000 you now`).
State is keyed by canonical path in `blame_gutters: HashMap<PathBuf,
BlameGutter>` — no parallel vector to keep aligned through open/close/session
restore, and closing a buffer cannot corrupt another's gutter. A gutter is
*valid* only while the buffer's edit epoch and line count still match the
capture; edits invalidate it (honest UI: a stale gutter refuses to render or
dive rather than lie). While data is not loaded the single-line blame card
remains the fallback: `Space g b` shows it immediately on toggle-on, and
Enter on an unloaded/stale gutter falls back to the card. With a valid gutter,
**Enter dives**: the commit browser opens *positioned at that line's SHA*
(`CommitLog.focus`, resolved when rows land). Uncommitted lines refuse with a
message. On readonly surfaces `Space g b` keeps the card (they have no path).

### 4. Commit-diff file sidebar (`]f` / `[f`)

A `Diff` surface reached from the changed-files dive carries
`CommitFiles { sha, files }` — the same typed numstat rows the files list
renders from (`show_stat`); no second data path. The renderer draws a
28-column left sidebar (`▌` + ellipsized path, current file on the selection
background, dim rule as the divider) before the diff gutter; structural bands
and full-row add/del pads stop at the sidebar's edge. `]f`/`[f` walk to the
next/previous file of the same commit (wraparound) by re-resolving
`commit_file_diff` into the *same* surface — label, hunks, stats and buffer
text are rewritten in place, cursor to top, return point untouched. Enter on
a sidebar row is deliberately not wired: the sidebar is a margin, outside the
cursor's grammar — a clickable-margin fork is the thing 0001 §4 forbids.

## Pushed back on (with reasons)

- **Re-blaming on edit automatically.** A background re-blame per keystroke is
  a job storm for zero archaeology value; invalidation + explicit re-toggle is
  one rule the user can see.
- **Sidebar as a navigable buffer** (motions into the sidebar): would need a
  second cursor per pane — a widget fork. `]f`/`[f` + highlight is the modal
  answer.
- **fzf-filter in the sidebar**: the changed-files surface (one `q` up) is
  already a searchable buffer.

## Files

- `strop-git/src/memory.rs` — `BlameLine`, `blame_file` (porcelain parser).
- `crates/strop/src/editor/git_memory.rs` — `CommitFiles`, `CommitLog.focus`,
  `BlameGutter` state + toggle/dive, `]f`/`[f`, generation-carrying jobs, `q`
  routing, unconditional restore helpers.
- `crates/strop/src/editor/{mod,normal,git}.rs` — generation counter +
  `blame_gutters` field, Enter dive dispatch, `Space g b` cutover.
- `crates/strop/src/render/{diff,buffer,mod}.rs` — blame column, sidebar
  spans, `left_inset` (cursor placement composed once, not per-surface).
