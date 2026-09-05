# 0015 — The trust release (0.6.1)

Status: accepted. The 2026-09-05 second review round verified the
restructure and found the second-order failures at the seams. This plan
is the P0 set — each item was verified against source or reproduced
headless before inclusion.

## Verified P0s (repro or source line)

1. **Ctrl-C bypasses dirty protection** — main.rs breaks the TUI loop
   directly; unsaved work dies without a word.
2. **Pathless save reports success** — `Buffer::save` on a pathless
   buffer returns `Ok(())`: `:w` says "written", `:wq` discards the
   buffer through what reads as a persistence command.
3. **A failed shell filter can erase text** — `selection | false` replaces
   the selection with the empty string; spawn failures insert error text
   INTO the document. Reproduced headless.
4. **`:wq` is not pane-aware** — closes the buffer out from under other
   panes holding its id (a live stale-DocumentId panic path).
5. **Hidden duplicate cursor** — the delete/change cascade plants the
   primary's own landing as an extra (`dw` leaves a stacked 2-cursor
   state at one position). Reproduced headless.
6. **`d0` broken** — the walker swallows `0` into count2 during
   op-pending; the op never completes. vim: `0` is a motion when no
   count digits are pending, a digit otherwise. Reproduced.
7. **Arrow keys bypass the walker's counts** — `2 <Right> x` moves once
   and then deletes 2 chars (the count leaks invisibly). Reproduced.
8. **Count overflow** — `count * 10 + digit` unchecked: debug panics,
   release wraps.
9. **Session history has no content identity** — dirty-history captured
   against unsaved text restores onto the disk version; undo replays
   against the wrong state. Fix: hash the text the history belongs to;
   on mismatch, restore the text and drop the history with a message.
10. **`Buffer::byte()` can panic on an empty rope** — clamps to 0 and
    reads byte 0 of an empty rope. Privatize; `byte_at` is the safe form.
11. **No `didClose`** — close/reopen in one process leaves the server
    holding stale text while strop suppresses the fresh didOpen.
12. **Walker display is incomplete** — pending op/register invisible.

## Pushbacks (recorded, not adopted)

- **`q` on readonly surfaces stays contextual.** vim's own special
  buffers remap normal keys (quickfix Enter, help Ctrl-], netrw's whole
  alphabet); strop's surfaces are visibly focused (modeline name +
  `[RO]`). Macros don't exist yet; when they land (0016), `q<reg>`
  records in normal buffers and surfaces keep their contextual layer.
- **Preview scope**: during operator-pending there is no target to show
  until the motion arrives, and the completing keystroke executes
  immediately — a "preview" of `dw` is one frame of nothing. The claim
  on the site narrows to what's true: preview and execution share the
  resolver/plan; search/refactor/replace/git previews show live.
- **No nested layout tree yet** (PaneId/ViewId arenas): the flat
  row/column layout is complete for v1; the arenas land WITH the tree.

## Deferred to later plans

- **0016** (0.7): the full typed input automaton — the walker emits
  `CommandId + typed args`, never an assembled string; macros; semantic
  model-based transition tests; alias→semantic-command mapping.
- **0017** (0.8): LineLayout + display columns + graphemes + visual
  block + pane-relative cursor geometry + bracketed paste.
- **0018** (0.9): revision-native git (byte-precise content, structured
  index mutation replacing textual patches, revision-keyed async
  snapshots, merge-base review) + service envelopes + per-workspace
  language config cache.

## The regression battery (all ship as tests)

```
dirty file → ctrl-c → warns once, second ctrl-c quits
dirty scratch → :w → "no file name"; :w path → writes+adopts path
dirty scratch → :wq → stays open
selection | false → text unchanged, stderr in message
selection | nonexistent-command → text unchanged
same doc in two panes → :wq → only the pane closes
dw → SelectionSet count stays 1
d0 → deletes to column 0
2 <Right> x → moves twice, x deletes one
count overflow input → capped, no panic
session: dirty history + changed disk → history dropped, text restored
close+reopen cpp → didClose then fresh didOpen
```

## Exit policy (the one rule)

Every exit path funnels through one decision: dirty documents block a
plain quit, with one explicit override gesture (a second ctrl-c, or
`:q!`). No exit path writes session state describing text it discarded.
