# 0064 — UI polish: per-pane scrollbars, Git overview, optional cursor fade

Status: **authorized** as the UI-polish slice of the pre-completion arc
([0063](0063-canonical-search-and-symbols.md) search, then this plan, then
0056/0057/0058 foundation work per the roadmap). Completion (0059), debugger
(0060) and GUI (0061) stay deferred until everything before them lands and is
bug-hardened; the GUI milestone owns any pixel work beyond this plan's
terminal-UI scope.

This document records requirements; it does not claim executed work.

## 1. Per-pane scrollbar with Git overview

Every editor pane's right border gains one thin, transparent-track scrollbar
carrying two aligned signals, with its column reserved before text budgets
are calculated ([0050](0050-picker-visual-polish-handoff.md) established the
reserved scrollbar vocabulary for pickers; this extends it to editor and
terminal panes):

- **Viewport position** — the pane's scroll range, from actual layout state,
  never a guess.
- **Git overview** — added/changed/deleted line spans of the visible
  document, aligned to the scrollbar's coordinate space, sourced from the
  existing hunk identity already maintained for diff review.

Contract:

- The scrollbar is one hint surface per pane; it never becomes an input owner
  and never overlaps the picker hint column conventions.
- Hunk-to-scroll mapping is monotonic and stable under resize; spans clamp to
  the viewport model, not to rendered rows, so wrapped and unwrapped layouts
  agree.
- Unavailable Git state renders an empty track, never a false neutral.
- Terminal panes show viewport position only (no Git overlay); their history
  bound is the projection's bounded history, not an illusion of infinity.

## 2. Optional slow cursor fading

The active cursor fades in over a short, bounded interval when input focus
returns to a pane (or the cursor jumps beyond a distance threshold), instead
of appearing instantly. The 0055 async invariant governs: the fade advances
on the existing render tick (`animation_due`), never an await or a thread,
and disables cleanly when the terminal reports no support for the required
frame pacing.

Contract:

- The fade is a presentation-only overlay on cursor state; replay checks and
  cell observations are unaffected (the cursor's final position is always the
  recorded one; partial-fade frames are not part of the forensic contract).
- One shared animation budget with the existing flash timings; no per-frame
  allocation in the input→render path.
- The feature is optional and off-able without touching grammar or input
  behavior.

## 3. Acceptance

- Golden cell-grid snapshots via `TestBackend` for: thin/wide panes, tiny
  recovery sizes, wrapped layouts, documents with dense and sparse changes,
  detached/no-Git documents, terminal panes with bounded history.
- Property: scrollbar geometry derives from the same layout owner the
  renderer consumes — no second layout pass, no divergence under resize
  storms (existing resize torture cases extend to assert scrollbar stability).
- Cursor fade: frame-sequence fixtures asserting monotonic progress, final
  state equals unfaded cursor, and zero effect when disabled.
- The repository gate (`docker compose run --build --rm test`) passes with
  all of the above; the 0063 search surfaces adopt the scrollbar vocabulary
  where they present scrollable results.

## Landed slices

### §1 scrollbar with Git overview: landed (2026-09-15/16)

Every editor and terminal pane reserves its rightmost column before any text
budget is computed (`render/buffer/scrollbar.rs`); prepare, admission and
paint share the one reserved budget (`render/prepare.rs`, `render/buffer.rs`),
so viewport geometry and cells cannot diverge. The track is quiet, the thumb
carries fractional viewport position over the document's own line space
(monotonic under resize; wrapped and unwrapped layouts agree), and the live
document's hunk signs paint sparse added/changed/deleted spans —
unavailable Git state renders the honest empty track, never a false neutral.
Terminal panes carry viewport position only, over the projection's bounded
history; their child grid narrows by the reserved column (pinned by `stty
size` = 28x59 in the split journey). Evidence: `cargo test` golden cell-grid
pins (`pane_scrollbar_carries_track_thumb_and_git_overview` — track, thumb,
fractional hunk mapping, thumb motion under caret-following scroll), the
updated resize/gutter/directory/markdown golden grids, and the full
real-terminal PTY journey whose row predicates are now track-aware (the
reserved cell broke exact full-row matches — a test-contract update, not an
editor defect; verified by thread-state capture of a healthy idle editor).
Still open from §3: picker surfaces adopting the vocabulary where they
present scrollable results.

### §2 cursor fade: landed (2026-09-16)

The Normal-mode block cursor fades in over 160 ms after focus returns,
pane/buffer switches and jumps beyond half a screen; ordinary typing and
small motions keep it steady (`editor/cursor.rs`, state in `Editor`
fields beside `flash`, retrigger bookkeeping after every recorded action
in `trace/drive.rs`). The fade advances on the existing 16 ms
`animation_due` tick shared with the flash — no thread, no await, no
per-frame allocation. While fading, the block is software-painted with a
linear RGB ramp and the native cursor stays hidden; the closing frame is
exactly the unfaded cursor (`render/mod.rs::place_cursor`, `fade_mix`).
Insert/replace/picker cursor styles and the embedded terminal's cursor
keep their owners. Progress derives from tape ticks, so recorded journeys
replay bit-identically (the trace test replays a faded journey; the
PTY replay lane covers the real renderer). `cursor_fade = false` in
config.toml disables the feature wholesale (knob in `config.rs`,
settings popup/`strop config` included). Terminals that cannot hold the
paint cadence degrade to fewer, larger steps with the identical final
frame — there is no terminal capability query for frame pacing, so the
off-switch is the knob. Evidence: engine retrigger/expiry/knob/replay
test (`cursor_fade_retriggers_only_on_focus_returns_and_large_jumps`),
render pin (`cursor_fade_paints_the_block_then_restores_the_unfaded_cursor`
— mid-fade interpolated cells, final frame equal to unfaded), full
workspace suite and clippy green.
