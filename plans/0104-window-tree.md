# 0104 — The window tree: splits that nest, as in vim

- Status: **proposed** (for 0.67). Hand-off plan: the implementer takes every
  default in §13 unless the owner answers otherwise.
- Source: the owner — "I did a horizontal split so now two windows, then doing
  a vertical split produced a third window but flipped everything into
  vertical. I feel it is buggy."
- Builds on [0103](0103-seams-and-splits.md) (title rules, the `│` divider,
  `┬`; it reserved `├ ┤ ┴ ┼` for this plan) and [0086](0086-verified-foundation.md)
  (ring 0: the arithmetic moves into `strop-kernel` and is proved in Lean).
  Amends 0103 §4.1 (the reserved junctions are now drawn) and the
  `panes are always equal` wording stays true.
- Oracle used while writing this: `nvim 0.9.5 --headless -u NONE` with
  `set splitbelow splitright`. A Python model of §3–§5 was run against it on
  more than 800 random and enumerated `ctrl-w` sequences (`s v c o w W h j k
  l`): the window tree (`winlayout()`) and the current window agreed in every
  one.
  Window sizes agree except for the odd cell and vim's `winwidth=20` (§5.4).

## 1. The bug and its cause

`:sp` then `:vs` shows three panes side by side. The first split's stacking is
gone.

The layout is one flag, not a tree:

- `crates/strop-engine/src/editor/panes.rs` — `enum LayoutDir { Row, Column }`
  ("v1 is a flat layout"), `Editor.layout: LayoutDir`, `Editor.panes: Vec<Pane>`.
  `split_document` pushes the new pane at the end and **overwrites** the flag:
  `self.layout = if vertical { Row } else { Column }`. So the last split decides
  the direction of every pane.
- `crates/strop-engine/src/editor/prepare/geometry.rs` — `pane_cells(count,
  is_row, area)` tiles `count` panes along one axis.
- `crates/strop/src/render/buffer.rs` — `render_panes` asks
  `editor.layout() == LayoutDir::Row` once for all panes.

Smaller faults that come from the same flag:

- `ctrl-w h/l` do nothing in a column and `j/k` nothing in a row, and all four
  wrap around (vim never wraps).
- A split made from a pane that is not the last one puts the new pane at the
  far end, not next to the pane that was split.
- `view_stamp` (`prepare.rs`) hashes `panes.len()` but not the layout.

## 2. Inventory: what strop has today

Kept in scope (all exist; all must work on the tree):

| Where | Commands |
|---|---|
| ex (`ex/table.rs`) | `:sp[lit] [file]`, `:vs[plit] [file]`, `:new`, `:vnew`, `:clo[se]`, `:on[ly]`, `:q`, `:q!`, `:qa` |
| `ctrl-w` (`panes.rs::window_command`) | `h j k l`, `w`, `W`, `p`, `s`, `v`, `n`, `c`, `q`, `o`, `=` (and `ctrl-w ctrl-{key}`) |
| `Space w` (`keymap/bindings/panes.rs`) | `h l j k`, `w`, `1`–`9`, `v`, `-`, `q` |
| other callers of `split_document` | `Space t` (terminal split below, `terminal/lifecycle.rs`), `OpenIntent::Split` (`io/admission.rs`), `ex_new` |

Not present, so nothing to do: mouse focus or mouse resize (no pointer code
reaches panes); the layout in sessions (`session.rs` restores buffers and the
jump list into whatever panes exist; it never saved the layout).

Stays on the Missing list (`keymap/missing.rs`, unchanged): `ctrl-w x r T H J
K L + - _ |`. See §12 for these and other nice-to-haves.

## 3. Semantics (checked against nvim)

strop's posture, unchanged: a new window opens **below** or to the **right**
and takes focus (vim's `splitbelow splitright`). `equalalways` is vim's
default, so vim re-equalises after every split and close; strop has no resize
command, so its layout is always the equalised tree (§5).

Windows are numbered in layout order, top-left first (vim's `winnr()`): the
tree's leaves read left-to-right, top-to-bottom. `panes[k]` is leaf `k`.

| Command | Effect on the tree | Focus |
|---|---|---|
| `:sp`, `ctrl-w s`, `Space w -`, `:new`, `ctrl-w n`, `Space t` | Split the **current** leaf along the column axis. If its parent is already a column, the new leaf becomes the next sibling. Otherwise the leaf is replaced by a column of two: itself, then the new leaf. | the new leaf (`active + 1`) |
| `:vs`, `ctrl-w v`, `Space w v`, `:vnew` | The same along the row axis. | the new leaf (`active + 1`) |
| `:clo`, `ctrl-w c`, `:q`/`ctrl-w q`/`Space w q` with 2+ panes | Remove the leaf. A node left with one child is replaced by that child; if that child is a split with the same axis as its new parent, its children are spliced into the parent. | The first leaf of the **previous** sibling; with no previous sibling, the first leaf of the next one. (vim gives the freed cells to that sibling.) |
| `ctrl-w c`, `:clo` on the last window | nothing; `cannot close the last window` | — |
| `:on`, `ctrl-w o` | The tree becomes one leaf. | that leaf |
| `ctrl-w w` / `W`, `Space w w` | none | next / previous leaf in number order, wrapping |
| `ctrl-w p` | none | the most recently focused other pane (unchanged) |
| `Space w 1`–`9` | none | that number |
| `ctrl-w h j k l`, `Space w h j k l` | none | the window that touches the current one on that side, at the cursor's row (`h l`) or column (`j k`); nothing on that side → nothing happens, no message, **no wrap** |
| `ctrl-w =` | none (sizes are always equal) | — |

Directional focus, exactly (vim's `win_horz_neighbor` / `win_vert_neighbor`,
restated on cells): let `F` be the current pane's frame (§5.1) and `(cx, cy)`
the cursor's cell inside it. The target is the pane whose frame, widened by
the divider column on its right, contains

- `j`: `(cx, F.y + F.height)`; `k`: `(cx, F.y − 1)`;
- `l`: `(F.x + F.width + 1, cy)`; `h`: `(F.x − 2, cy)`.

A subtraction that would go below zero, or a point no frame contains, means
"no window there". A divider column belongs to the frame on its left, as vim's
separator does.

## 4. Data model

### 4.1 Engine

New file `crates/strop-engine/src/editor/panes/layout.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis { Row, Column }      // Row: side by side. Column: stacked.

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Layout {
    Leaf,
    Split { axis: Axis, children: Vec<Layout> },
}
```

`LayoutDir` is deleted (no alias). `Editor.layout: Layout`, starting as
`Layout::Leaf`. Leaves hold no id: leaf `k` in depth-first order is
`Editor.panes[k]`.

Invariants, checked by `Layout::well_formed()` and `debug_assert`ed after
every structural change:

1. a `Split` has at least two children;
2. no child `Split` has its parent's axis (normal form);
3. `layout.leaves() == panes.len().max(1)`.

Methods (all on `Layout`, all pure, none `pub` beyond the crate except
`outline` and `leaves`):

| Method | Meaning |
|---|---|
| `leaves() -> usize` | leaf count |
| `tracks(axis) -> usize` | leaf: 1; split along `axis`: sum of children; split across it: max of children (vim's "windows across") |
| `split(leaf, axis)` | §3 row 1/2; the new leaf is `leaf + 1` |
| `remove(leaf) -> usize` | §3 row 3; returns the leaf to focus, in the numbering after removal |
| `place(area) -> Placement` | §5 |
| `outline() -> serde_json::Value` | leaf → its 1-based number; split → `["row" \| "col", [children…]]`. The same shape as vim's `winlayout()` with window numbers. Used by `state_json`, the trace snapshot and every test that asserts a shape. |

`split` and `remove` walk a path of child indexes from the root to leaf
`leaf` (one private helper, `path(leaf) -> Vec<usize>`).

- `split`: empty path (root is a leaf) → root becomes `Split { axis, [Leaf,
  Leaf] }`. Else let `parent` be the node at `path[..last]` and `i =
  path[last]`: if `parent.axis == axis`, insert a `Leaf` at `i + 1`; else
  replace child `i` with `Split { axis, [Leaf, Leaf] }`.
- `remove` (precondition `leaves() >= 2`): with `parent`, `i` as above, the
  focus is `leaf − children[i−1].leaves()` when `i > 0`, else `leaf`. Remove
  child `i`. If one child `only` remains, the parent node is replaced by
  `only`; if `only` is a `Split` and the replaced node had a parent, `only`'s
  children are spliced into that grandparent at the node's position (by
  invariant 2 the axes are equal exactly then).

`Editor` changes pane structure only through three private methods in
`panes/mod.rs`, each ending in `debug_assert!(self.layout_matches_panes())`:

- `insert_pane(axis, pane)` — `layout.split(active, axis)`,
  `panes.insert(active + 1, pane)`, stamp the old pane's `focused`,
  `active_pane += 1`, `focus_epoch += 1`;
- `remove_active_pane()` — `let focus = layout.remove(active)`,
  `panes.remove(active)`, `active_pane = focus`, `focus_epoch += 1`;
- `keep_only_active_pane()` — one pane, `Layout::Leaf`, `active_pane = 0`.

`document/close.rs` (`self.panes.clear()` when the last document closes) also
sets `self.layout = Layout::Leaf`. `window_command('o')` and `ex_only` call
`keep_only_active_pane` (today they are two copies; `ex_only` also runs
`discover_git` — keep that in the shared method).

### 4.2 Kernel: one axis, flat slices

The tree stays in the engine. What moves into `strop-kernel` is the decision
that can be wrong in ways a test will not exhaust: how one node's cells are
divided among its children. New module `crates/strop-kernel/src/layout.rs`,
shaped like `cache.rs` (slices the caller owns, `while` loops, no `Vec`, no
recursion, no `Box`):

```rust
/// One child's cells along its parent's axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span { pub at: usize, pub len: usize }

/// What the owner fills `out` with before `tile`.
pub fn empty() -> Span { Span { at: 0, len: 0 } }

/// The fewest cells `tracks` windows side by side need: `unit` each and
/// `gap` between neighbours.
pub fn minimum(tracks: usize, unit: usize, gap: usize) -> usize {
    if tracks == 0 { 0 } else { tracks * unit + (tracks - 1) * gap }
}

/// The windows across all children.
pub fn total(tracks: &[usize]) -> usize { /* while-loop sum */ }

/// A child of `tracks` windows takes this much of `room` spare cells while
/// `weight` windows are still to be served (itself included). Rounds down:
/// the odd cells go to later children.
pub fn share(room: usize, tracks: usize, weight: usize) -> usize {
    if weight == 0 { 0 } else { tracks * room / weight }
}

/// Divide `total` cells among children. Child `k` holds `tracks[k]` windows
/// side by side. Every child first gets its minimum, then the spare cells
/// are shared by window count; the last child takes what is left. When the
/// minimum does not fit, earlier children keep theirs and later ones shrink
/// to nothing; no span ever passes `total`.
pub fn tile(total: usize, unit: usize, gap: usize, tracks: &[usize], out: &mut [Span]) {
    let mut weight = self::total(tracks);
    let need = minimum(weight, unit, gap);
    let mut room = if total > need { total - need } else { 0 };
    let mut at = 0;
    let mut k = 0;
    while k < tracks.len() {
        if k >= out.len() { return; }
        let left = total - at;
        let len = if k + 1 == tracks.len() {
            left
        } else {
            let extra = share(room, tracks[k], weight);
            room -= extra;
            weight -= tracks[k];
            least(minimum(tracks[k], unit, gap) + extra, left)
        };
        out[k] = Span { at, len };
        at = least(at + len + gap, total);
        k += 1;
    }
}
```

`least(a, b)` is a private three-line minimum (`excerpt::clamp` is private to
its module and its name is in `proofs/Kernel/Excerpt.lean`; do not move it).
Rename the `total` parameter or the function if the shadowing troubles Charon
(`cells` for the parameter is fine).

This is vim's `win_equal_rec` with `winwidth`/`winheight` left out: minimums
first (`frame_minwidth`), spare room by windows across (`wincount * room /
totwin`), the last frame takes the rest.

## 5. Geometry

### 5.1 Frames, panes, dividers

`Layout::place(area: CellRect) -> Placement`, in `panes/layout.rs`, replaces
`pane_cells`:

```rust
pub struct Placement {
    pub panes: Vec<CellRect>,     // text rects, title row already taken off
    pub dividers: Vec<CellRect>,  // width 1; one per boundary between row children
}
```

- `area` is the whole view; the last row is the modeline, as today. The
  canvas is `(area.x, area.y, area.width, area.height − 1)`.
- One leaf: `panes = [canvas]`, no title, no dividers (today's result).
- Otherwise each node gets a **frame** (a pane plus its title row), top down
  from the canvas:
  - `Split { Row }` with frame `(x, y, w, h)`: `tile(w, 1, 1, tracks_row,
    out)` where `tracks_row[k] = children[k].tracks(Row)`. Child `k`'s frame
    is `(x + out[k].at, y, out[k].len, h)`. For every `k` but the last, a
    divider `(x + out[k].at + out[k].len, y, 1, h)` when that column is
    inside the frame.
  - `Split { Column }`: `tile(h, 2, 0, tracks_col, out)`; child frame `(x,
    y + out[k].at, w, out[k].len)`. No divider, no gap: the lower title is
    the boundary (0103 §5.4). `unit` is 2 because a stacked pane needs its
    title row and one text row.
  - `Leaf` with frame `f`: if `f.width > 0 && f.height > 0` the pane is `(f.x,
    f.y + 1, f.width, f.height − 1)` (title on row `f.y`), else `f` unchanged
    — the same rule `pane_cells` has today.
- Values are `usize` in the kernel; narrowing to `u16` happens once, here.
  Every span is inside its parent (theorem §9.1), so the narrowing cannot
  truncate.

### 5.2 What does not change

For the layouts strop could already show, `place` returns exactly what
`pane_cells` returned wherever the remainder is at most one cell: two panes
side by side in 80 columns are 39 and 40; two stacked in 24 rows are `(y 1, h
10)` and `(y 12, h 11)`; three in 80 columns are 26, 26, 26. So the existing
`split-*`, `terminal-*` goldens must not change. One improvement: `pane_cells`
gave the whole remainder to the last pane (80 cells among 3 with no gaps
would be 26, 26, 28); `tile` spreads it (26, 27, 27).

### 5.3 The owner's case, in numbers

80×24, `:sp` then `:vs`: `["col",[1,["row",[2,3]]]]`. Frames: 1 `(0,0,80,11)`;
2 `(0,11,39,12)`; 3 `(40,11,40,12)`. Text rects: 1 `(0,1,80,10)`; 2
`(0,12,39,11)`; 3 `(40,12,40,11)`. One divider `(39,11,1,12)`. Neovim gives
the same three windows the same cells.

### 5.4 Where sizes differ from vim, and why that is accepted

- Vim rounds half up and strop rounds down (strop today, and Neovim when the
  new window is the current one, both give 39 | 40).
- Vim gives the current window at least `winwidth` (20) columns, so five
  side-by-side windows in 80 columns are 14 14 14 14 20 in vim. Sizes that
  depend on where the cursor was when you split are not something strop wants.
- Vim equalises only the split's own direction after a split or close, so a
  cell of rounding can survive from an earlier layout.

The tree and the focus are the grammar and must match; the odd cell is not.
The differential harness therefore compares shape and current window, not
sizes (§8.4).

### 5.5 Directional focus without a prepared frame

`Editor::pane_towards(key) -> Option<usize>` (in `panes/focus.rs`) implements
§3. It places the layout on `self.prepared.geometry` **widened per axis to the
tree's minimum** — `max(columns, minimum(tracks(Row), 1, 1))` and `max(rows −
1, minimum(tracks(Column), 2, 0))` — so every window has cells even in a tiny
terminal or before the first prepare (engine unit tests never prepare). One
formula; no "geometry unknown" branch.

The cursor's cell is `cx = F.x + min(left_inset + cell_column − hscroll,
F.width − 1)`, `cy = F.y + 1 + min(line_of(head) − view_top, F.height − 2)`,
using the same `left_inset` and `try_cell_col_with_tab` that
`adjust_active_viewport` uses; when the bounded column probe declines (cold
layout on a long line) `cx = F.x`. Saturating arithmetic throughout.

## 6. The look (0103, nested)

Rules, all from 0103 §4 — nothing new but the four reserved glyphs:

1. Every pane (two or more on screen) starts at its title rule `─ N name ───`,
   as wide as the pane. It takes that pane's focus ink (`fg.muted` focused,
   `fg.faint` not).
2. Every row split has a divider column between neighbours, the full height
   of **that split's frame** — not of the screen. `fg.faint` always.
3. A divider cell at row `y` is drawn from three facts:
   - `left`: a painted title ends against it (a pane with `rect.y − 1 == y`
     and `rect.x + rect.width == x`, non-empty rect);
   - `right`: a painted title starts after it (`rect.y − 1 == y`, `rect.x ==
     x + 1`);
   - `up`: the divider has a cell above (`y > divider.y`), or another divider
     in the same column ends exactly here.

   | left | right | up | glyph |
   |---|---|---|---|
   | no | no | any | `│` |
   | any, at least one | | no | `┬` |
   | yes | yes | yes | `┼` |
   | no | yes | yes | `├` |
   | yes | no | yes | `┤` |

4. Where a divider ends on a title rule below it (the cell under its last row
   is above the modeline and is not itself a divider cell), that title cell
   becomes `┴` **if it is a `─`**. Title text wins over the junction: a name is
   never overwritten. `┴` keeps the title's ink (0103 §4.3: a line never
   changes colour because of what is above it). `┬ ├ ┤ ┼` are in the divider
   column and are `fg.faint`.
5. No corners, ever (0103).

Mock-ups at 64×12 (generated from the model, so the columns are the real
arithmetic).

**(a) The owner's case** — `:sp`, `:vs`; pane 3 focused:

```
─ 1 src/statusline.rs ──────────────────────────────────────────
   1 //! The modeline: one quiet row at the bottom edge.
   2 //!
   3 //! Every width is a display cell.
   4
─ 2 src/statusline.rs ─────────┬─ 3 src/statusline.rs ──────────
   1 //! The modeline: one quie│   1 //! The modeline: one quiet
   2 //!                       │   2 //!
   3 //! Every width is a displ│   3 //! Every width is a displa
   4                           │   4
   5 use std::path::Path;      │   5 use std::path::Path;
 NORMAL  src/statusline.rs                                  1:1
```

Title 1 runs the full width in `fg.faint`. The divider starts at the lower
titles with `┬` and stops at the modeline. Title 3 is `fg.muted`.

**(b) An L** — `:vs`, `:sp` (`["row",[1,["col",[2,3]]]]`):

```
─ 1 src/statusline.rs ─────────┬─ 2 src/main.rs ────────────────
   1 //! The modeline: one quie│   1 //! The modeline: one quiet
   2 //!                       │   2 //!
   3 //! Every width is a displ│   3 //! Every width is a displa
   4                           │   4
   5 use std::path::Path;      ├─ 3 src/main.rs ────────────────
   6                           │   1 //! The modeline: one quiet
   7 pub struct Modeline {     │   2 //!
   8     chip: String,         │   3 //! Every width is a displa
   9 }                         │   4
  10                           │   5 use std::path::Path;
 NORMAL  src/main.rs                                        1:1
```

`├`: title 3 leaves the divider to the right; pane 1's text is on the left.
The mirror, `["row",[["col",[1,2]],3]]`, draws `┤`:

```
─ 1 src/main.rs ───────────────┬─ 3 src/statusline.rs ──────────
   1 //! The modeline: one quie│   1 //! The modeline: one quiet
   4                           │   4
─ 2 src/main.rs ───────────────┤   5 use std::path::Path;
   1 //! The modeline: one quie│   6
```

**(c) Four ways** — `:sp`, `ctrl-w k`, `:vs`, `ctrl-w j`, `:vs`
(`["col",[["row",[1,2]],["row",[3,4]]]]`):

```
─ 1 a.rs ──────────────────────┬─ 2 b.rs ───────────────────────
   1 //! The modeline: one quie│   1 //! The modeline: one quiet
   2 //!                       │   2 //!
   3 //! Every width is a displ│   3 //! Every width is a displa
   4                           │   4
─ 3 c.rs ──────────────────────┼─ 4 d.rs ───────────────────────
   1 //! The modeline: one quie│   1 //! The modeline: one quiet
   2 //!                       │   2 //!
   3 //! Every width is a displ│   3 //! Every width is a displa
   4                           │   4
   5 use std::path::Path;      │   5 use std::path::Path;
 NORMAL  d.rs                                               1:1
```

Two dividers in one column, the upper ending where the lower begins: `┼`.
`["row",[["col",[1,2]],["col",[3,4]]]]` (one divider, titles on both sides at
the same row) draws the same lines, with the panes numbered down each column.

**(d) Dividers that do not line up** — three over two:

```
─ 1 a.rs ───────────┬─ 2 b.rs ────────────┬─ 3 c.rs ────────────
   1 //! The modelin│   1 //! The modeline│   1 //! The modeline
   4                │   4                 │   4
─ 4 d.rs ───────────┴──────────┬─ 5 e.rs ─┴─────────────────────
   1 //! The modeline: one quie│   1 //! The modeline: one quiet
```

Each upper divider lands on a title rule: `┴`, in that title's ink. Had `e.rs`
been a longer name, the second `┴` would fall on a letter and not be drawn.

**(e) Three levels** — `["row",[1,["col",[["row",[2,3]],4]]]]`:

```
─ 1 a.rs ───────────┬─ 2 b.rs ────────────┬─ 3 c.rs ────────────
   1 //! The modelin│   1 //! The modeline│   1 //! The modeline
   5 use std::path::├─ 4 d.rs ────────────┴─────────────────────
   6                │   1 //! The modeline: one quiet row at the
```

Vim agrees on the widths here (three equal columns): a child's width is by how
many windows it has across, not by how many children its parent has.

## 7. Implementation, in landing order

Each commit passes `docker compose run --build --rm test`. Commit 5 also needs
the `proofs` gate.

### Commit 1 — kernel: `strop_kernel::layout` (code and translation, no theorems yet)

- `crates/strop-kernel/src/layout.rs`: §4.2, with a module doc in the style of
  `cache.rs` ("Proved (proofs/Kernel/Layout.lean): …" listing §9.1), and unit
  tests: the numbers of §5.2 and §5.3; a loop over `total` 0..=200 × small
  track lists asserting in-bounds, order, and (when it fits) exact cover.
- `lib.rs`: `pub mod layout;`. `Cargo.toml`: add `"strop_kernel::layout"` to
  `[package.metadata.charon] start_from` (the gate refuses a `pub mod` missing
  from it).
- `sh proofs/refresh.sh`; commit `proofs/Extracted/`. **Do this first**, before
  writing anything else: it shows at once whether Charon/Aeneas accept the
  shape. If the early `return` inside the loop or the nested `if` in `tile`
  is rejected, restructure (e.g. `while k < tracks.len() && k < out.len()`
  as two nested conditions, or a helper `len_for(...)`) until it translates;
  the code in §4.2 is the reference behaviour, not sacred syntax.

### Commit 2 — engine and paint: the tree

`crates/strop-engine/src/editor/panes.rs` becomes a directory (it is 455 lines
and gains about 300):

- `panes/mod.rs` — `Pane`, the `Editor` accessors, `split`, `split_document`,
  `close_pane_or_buffer`, `quit_all`, the three structural methods of §4.1,
  `window_command`, the table shims.
  - `split_document(vertical, doc)`: build the pane as today, then
    `insert_pane(if vertical { Axis::Row } else { Axis::Column }, pane)`.
  - `close_pane_or_buffer`: `remove_active_pane()` in place of
    `panes.remove` + `min`.
  - `Editor::layout() -> &Layout` replaces the `LayoutDir` accessor.
- `panes/layout.rs` — §4.1 and §5.1 (`Axis`, `Layout`, `Placement`, `place`).
- `panes/focus.rs` — `pane_move`, `focus_pane`, `pane_towards` (§5.5).
  `pane_move`: digits and `w` as today; `h j k l` call `pane_towards` and do
  nothing when it is `None`. `window_command` keeps delegating `h j k l w`.
  Update the two doc comments that say "Directions wrap".
- `panes/tests.rs` — the existing tests, adapted (`LayoutDir` asserts become
  `e.layout().outline().to_string()`), plus §8.1.
- `editor/mod.rs`: field `layout: Layout`, init `Layout::Leaf`, re-export
  `pub use panes::{Axis, Layout, Pane, Placement};`, the field's doc comment.
  `state_json` gains `"layout": editor.layout.outline()`.
- `editor/prepare/geometry.rs`: delete `pane_cells` and its two tests (they
  move, rewritten, to `panes/tests.rs`).
- `editor/prepare.rs`: `let placed = self.layout.place(CellRect::from(geometry));`
  `rects = placed.panes`; `PreparedView` gains `pub dividers: Vec<CellRect>`
  (set from `placed.dividers`, empty in `PreparedView::empty`). `view_stamp`
  hashes `self.layout`. Nothing else in preparation changes: terminal PTY
  resize, `view_rows`, reveal and admission already key on each pane's rect.
- `editor/document/close.rs`: reset the layout with the panes (§4.1).
- `editor/ex/files.rs`: `ex_only` → `keep_only_active_pane`.
- `editor/git/tests.rs:184`: `e.panes.push(..)` → `split_document`.
- `editor/trace/seed.rs`: `layout: Layout`; `SEMANTIC_VERSION` 6 → 7 with a
  comment line ("Version 7: the window tree; panes are in layout order");
  `validate` refuses a seed whose layout is not `well_formed` or whose leaf
  count differs from `panes.len()`.
- `editor/trace/snapshot.rs`: add `"layout": self.layout` beside `"panes"`.
- `editor/transaction_conformance/service.rs`: the model follows. Its splits
  are all vertical, so the model stays a list: `Split` → `model.panes.insert(
  model.active + 1, *slot); model.active += 1;` `ClosePane` (2+ panes) →
  `model.panes.remove(model.active); model.active =
  model.active.saturating_sub(1);`. The `must!` checks stay as they are.
- `crates/strop/src/render/buffer.rs`: move `render_title_rule` and
  `render_divider` to a new `render/buffer/chrome.rs` and add the junctions
  there:
  - `render_panes` paints titles and pane bodies as today (drop `is_row` and
    the per-pane divider call), then calls `chrome::render_dividers(t, frame,
    area, prepared)`.
  - `render_dividers`: for each `prepared.dividers` cell choose the glyph with
    a pure `fn junction(left: bool, right: bool, up: bool) -> &'static str`
    (§6 rule 3) from `prepared.panes` rects and `prepared.dividers`; then
    rule 4: the cell under a divider's last row, if above the modeline row,
    not in any divider, and currently `─`, becomes `┴` (symbol only).
  - Paint never calls `place` (AR03): it reads `prepared` only.
  - Module doc of `buffer.rs` ("Chrome …") gains the junction sentence.
- Callers and tests that assumed "the new pane is last" or "close focuses the
  next index" — run the suite and adapt; known sites: `editor/jumps.rs` tests,
  `completion/context/tests.rs`, `io/remote_tests.rs`,
  `terminal/pty_tests.rs:258` (`LayoutDir::Column` → outline
  `["col",[1,2]]`), `render/buffer/tests/{identity,viewport}.rs`,
  `render/terminal_tests.rs`.
- Pane indexes now shift on a split as well as on a close. Audit the three
  places that store one — `PromptOrigin.pane_index` (`pending.rs`),
  `ResolutionKey.pane`, `CompletionScope.pane` — and confirm each is dropped or
  re-keyed by `cancel_pending` / `focus_epoch` on a split (they are today for
  a close). If one is not, fix it there; do not add an index-remap layer.

### Commit 3 — goldens

`render/golden_tests.rs`, with `each` (two themes × two sizes), driven by
keys so the goldens also pin the grammar:

- `split-nested` — `:sp<cr>:vs<cr>` (§6 a), pane 3 active;
- `split-ell` — `:vs<cr>:sp<cr>` (§6 b), `├`;
- `split-grid` — `:sp<cr><c-w>k:vs<cr><c-w>j:vs<cr>` (§6 c), `┼`;
- `split-offset` — `:sp<cr>:vs<cr><c-w>k:vs<cr>:vs<cr>`: three over two
  (§6 d), `┴`.

The existing `split`, `split-stacked`, `split-three`, `split-collection`,
`terminal*` goldens must come out byte-identical; if one changes, the
arithmetic of §5.2 is wrong — fix the code, do not re-record.

### Commit 4 — differential: windows against Neovim

§8.4.

### Commit 5 — proofs

§9: `proofs/Kernel/Layout.lean`, `proofs/Kernel.lean` import, `Audit.lean`
claims, `proofs/mutants.py`. Gate: `docker compose --profile proofs run --rm
proofs`.

### Commit 6 — words

- `docs/editor.md` (≈ lines 144–148 and 321–325): splits nest; a split divides
  the current pane; numbers follow the layout; `ctrl-w h j k l` go to the
  window on that side and stop at the edge; the junction glyphs.
- `keymap/bindings/panes.rs`: descriptions only where they are now wrong
  (`window-focus` is fine; `window-equal` stays "panes always are").
- `CHANGELOG.md` under 0.67.0, plain words: "Splits nest. `:sp` then `:vs`
  divides the pane you are in and leaves the others where they were…"; "the
  new pane sits next to the one you split and panes are numbered in reading
  order"; "`ctrl-w h/j/k/l` go to the pane on that side and no longer wrap";
  "closing a pane gives its space to its neighbour".
- `plans/STATUS.md`: a row for 0104; this file's status line → "in force
  (0.67)"; `plans/0103` §4.1 gets a one-line "junctions drawn by 0104".
- The head comment of `panes/mod.rs` (today "v1: a flat row… mixed nesting is
  the tree-layout follow-up") is rewritten.

## 8. Tests

### 8.1 Unit (`panes/tests.rs`)

- The bug: `:sp<cr>:vs<cr>` → outline `["col",[1,["row",[2,3]]]]`, active 2.
- Each row of §3 as one test: outline and `active_pane` after the keys
  (`:vs`,`:sp` → L; sibling insert `:vs`,`:vs` → `["row",[1,2,3]]`; split
  from a middle pane lands at `active + 1`; close with and without a previous
  sibling; close that collapses a node; close that splices
  (`["row",[1,["col",[["row",[2,3]],4]]]]` close 4 → `["row",[1,2,3]]`);
  `:only` from a nested tree).
- `ctrl-w h j k l`: on the grid, each of the four from each pane; no wrap at
  every edge; from the wide lower pane of `["col",[["row",[1,2]],3]]`, `k`
  goes to 1 with the cursor in column 0 and to 2 with the cursor at the end of
  a 70-character line (after a `prepare_view` at 80×24).
- Existing: `every_ctrl_w_key_is_a_window_command_or_reports_unbound`
  unchanged (`BOUND` is the same set).
- Geometry: the numbers of §5.2 and §5.3; `place` of one leaf is the canvas.

### 8.2 Property (`panes/tests.rs`, plain loops — no new dependency)

A tiny deterministic generator (an LCG seeded per case, 2 000 cases): start
from one pane, apply 1–12 random operations from {split row, split column,
close, focus next}, mirroring the count in a `usize`. After every operation:

- `well_formed()`, `leaves() == panes.len()`, `active_pane < panes.len()`;
- a split adds exactly one pane and focuses `old_active + 1`; a close removes
  exactly one and leaves the others in their relative order;
- **tiling**: for areas 1×1 … 100×40 (step so the run stays under a second),
  paint a `Vec<u8>` cell grid: +1 for every pane cell, every title cell (row
  `rect.y − 1` over the pane's width, non-empty panes only) and every divider
  cell. No cell above the modeline is counted twice, none lies on or below the
  modeline row, and when the area is at least the tree's minimum on both axes
  every cell is counted exactly once and every pane has `width ≥ 1` and
  `height ≥ 1`.

This is the tree-level tiling statement. The kernel theorems (§9.1) give it
per node; the structural induction over the tree is checked here, not in Lean
(§9.3).

### 8.3 Paint (`render/buffer/tests/identity.rs`)

- `junction` truth table (rule 3), one assert per row.
- On §6 (a)–(d) at 80×24: the exact cell of each junction holds the expected
  glyph; every divider and junction but `┴` is `t.faint`; `┴` has the fg of the
  title cell beside it; swapping focus changes no symbol.
- No `╭ ╮ ╰ ╯` in any nested layout at 80×24, 48×16, 24×10 (extends the 0103
  test).
- A title whose name covers the column under a divider keeps its letter.

### 8.4 Differential (`crates/strop/tests/differential/`)

- `main.rs`:
  - `State` gains `windows: String` (shown in `Display`).
  - Neovim: add `+set splitbelow splitright` beside the existing `+set`
    arguments (strop's posture, like `shiftwidth=4 expandtab`); write a
    `win.vim` into the case directory defining
    `function! L(l)` — `return a:l[0] ==# 'leaf' ? win_id2win(a:l[1]) :
    [a:l[0], map(copy(a:l[1]), 'L(v:val)')]` — and pass `+source win.vim`
    before the driver; the dump adds `'WIN:'.json_encode([L(winlayout()),
    winnr()])`, parsed like `REGS:`.
  - strop: `windows` is `[v["layout"], v["active_pane"] + 1]` from the state
    JSON. Both sides are normalised through `serde_json::Value::to_string`.
- New `windows.rs`, prefix `win/`, text `"alpha\nbeta\ngamma\n"`:
  - hand cases, named: the bug (`<c-w>s<c-w>v`), `:sp<cr>:vs<cr>`,
    `:vs<cr>:sp<cr>`, the grid, three-over-two, each close rule of §3,
    `:only`, `:close` on the last window, `<c-w>w`/`W` wrap, no-wrap at an
    edge, an edit after moving (`<c-w>s<c-w>v<c-w>kdd` — text and cursor are
    compared too);
  - generated: `<c-w>s<c-w>v` followed by every 3-key sequence over
    `s v c h k` as `<c-w>{key}` (125 cases). The model agreed with Neovim on
    all 125, and on a sample of the wider `s v c h j k l w` family.
  - Not in the corpus, with the reason in the file's head comment: `<c-w>n`
    and `:new` (the harness ends with `:w`, and the new buffer has no name);
    `<c-w>p` after a close (§12); anything where the cursor column decides
    `j`/`k` — headless Neovim does not redraw between fed keys, so its
    `w_wcol` is stale and the oracle answers for column 0. The corpus keeps
    the cursor in column 0 for directional cases; the column rule is pinned by
    the unit test in §8.1.
- `divergences.rs`: raise `MINIMUM_CASES` by the number of cases added. No
  new `KNOWN` entry is
  expected. If the Docker image's Neovim disagrees with 0.9.5 on focus after
  close, the oracle wins: change `Layout::remove`'s focus rule and §3 to
  match, do not list a divergence.

### 8.5 Trace, replay, headless

- `state` (headless) and the trace `State` event carry `layout`; replay
  compares it like any other field. The seed's version bump makes older tapes
  refuse to replay instead of replaying with pane numbers that mean something
  else. No fixtures in the repo carry a seed (`grep semantic_version` finds
  only `seed.rs`).
- One seed round-trip test: capture → restore keeps `layout`; a seed with a
  two-leaf layout and one pane is refused.

### 8.6 Conformance model

`transaction_conformance` (§7 commit 2). Its generated recipes exercise
`Split`/`ClosePane` interleaved with document closes; the `must!`s on pane
count and active pane are the check that the model's two new formulas are the
engine's.

## 9. Proofs

### 9.1 Theorems (`proofs/Kernel/Layout.lean`, namespace `Strop.Kernel`)

Write a pure Lean model first, as `EditMap.lean` does with `mapSpec`:

```lean
def minSpec (t unit gap : Nat) : Nat := if t = 0 then 0 else t * unit + (t - 1) * gap

/-- `tile`'s loop from child `k` on: (at, len) per child. -/
def tileFrom (cells unit gap : Nat) : Nat → Nat → Nat → List Nat → List (Nat × Nat)
  | _,  _,    _,      []        => []
  | at, _,    _,      [_]       => [(at, cells - at)]
  | at, room, weight, t :: rest =>
      let extra := t * room / weight
      let len := min (minSpec t unit gap + extra) (cells - at)
      (at, len) :: tileFrom cells unit gap (min (at + len + gap) cells) (room - extra) (weight - t) rest
```

`tile_spec` proves the extracted `layout.tile` returns exactly `tileFrom …`
written into `out` (mechanical: one loop invariant). Everything below is then
ordinary induction over a list, with no Aeneas monad in sight.

Hypotheses shared by all (`Bounded`): `cells ≤ 65535`, `tracks.sum ≤ 65535`,
`unit ≤ 2`, `gap ≤ 1` — so no product or sum can overflow even a 32-bit
`usize` (`65535² < 2³²`). Production satisfies them: cells are `u16`, a tree
has fewer leaves than cells that matter, and the engine passes only `(1, 1)`
and `(2, 0)`. `Fits` is `minSpec tracks.sum unit gap ≤ cells ∧ ∀ t ∈ tracks,
1 ≤ t ∧ tracks ≠ []`.

| # | Claim (name in `Audit.lean`) | Statement | What it means on screen |
|---|---|---|---|
| T1 | `tile_in_bounds` | Under `Bounded`, for every `k`: `out[k].at + out[k].len ≤ cells`, and `out[k].at + out[k].len ≤ out[k+1].at`. No `Fits` needed. | Panes never overlap and nothing is drawn past the frame — so nothing reaches the modeline — at any terminal size. |
| T2 | `tile_exact` | Under `Fits`: `out[0].at = 0`; `out[k+1].at = out[k].at + out[k].len + gap`; the last span ends at `cells`. | Children and their dividers partition the parent exactly: no stray column, no lost row. Closing a child and re-tiling therefore hands its cells back to the others. |
| T3 | `tile_minimum` | Under `Fits`: `out[k].len ≥ minSpec tracks[k] unit gap`. | When the terminal is big enough, every window gets at least one column, and a title plus one row. |
| T4 | `tile_fair` | Under `Fits`, with `q = (cells − minSpec tracks.sum unit gap) / tracks.sum`: `minSpec tracks[k] unit gap + tracks[k]·q ≤ out[k].len ≤ minSpec tracks[k] unit gap + tracks[k]·(q+1)`. Corollary `tile_fair_leaves`: if every `tracks[k] = 1`, any two lengths differ by at most 1. | `ctrl-w =` holds by construction: side-by-side windows of one split differ by at most one cell, and a nested group gets its share by window count. |

Also audited, small: `minimum_spec` (`= minSpec`), `total_spec` (the sum),
`share_spec` (`= tracks * room / weight`, 0 when `weight = 0`).

T4's proof is the only one with real arithmetic. The invariant: with `R`,
`W` the room and weight still to serve, `q·W ≤ R ≤ (q+1)·W`. It holds at the
start (`q = R / W`), each step keeps it (`⌊t·R/W⌋` is between `t·q` and
`t·(q+1)`, and `R − ⌊t·R/W⌋ ≤ ⌈R·(W−t)/W⌉ ≤ (q+1)(W−t)`), and the last child
takes `R` with `W = t`. Use `Nat.div_mul_le_self`, `Nat.lt_div_mul_add`,
`Nat.le_div_iff_mul_le` and `nlinarith`/`omega` on the multiplied-out forms.

Budget and the rule for coming up short: T1–T3 with `tile_spec` are expected
to take about the size of `Cache.lean` (≈ 500 lines). They are required for
the release. T4 is expected to add 150–250 lines. If T4 is not closed, do not
weaken it and do not `sorry` it: leave `tile_fair` and `tile_fair_leaves` out
of the file and the claims, add a fairness check to the §8.2 property test
(the panes of one flat split differ by at most one cell) as the cover, and say
so in this plan's status line. Nothing else in the plan depends on T4; every
mutant in §9.2 stays.

### 9.2 Wiring

- `proofs/Kernel.lean`: `import Kernel.Layout`.
- `proofs/Audit.lean` `claims`: `minimum_spec`, `total_spec`, `share_spec`,
  `tile_spec`, `tile_in_bounds`, `tile_exact`, `tile_minimum`, `tile_fair`,
  `tile_fair_leaves`; the doc comment on `claims` gains "plans/0104 §9".
- `proofs/mutants.py` — every mutant must make the build fail inside the
  theorem named and nowhere else. The names below are the fact each breaks;
  as with the existing mutants, the `fails` tuple names whichever spec
  theorem actually carries that step (most likely `tile_loop_spec` or
  `tile_spec`, since the facts about the pure model do not see the Rust) —
  record what the calibration run reports.

| Mutant | Change in `layout.rs` | Breaks |
|---|---|---|
| `layout-gap-dropped` | `least(at + len + gap, total)` → `least(at + len, total)` | T2: the next child starts on the divider |
| `layout-minimum-forgotten` | `least(minimum(tracks[k], unit, gap) + extra, left)` → `least(extra, left)` | T3 |
| `layout-overrun` | that same line → `minimum(tracks[k], unit, gap) + extra` (no `least`) | T1: a span passes the frame when it does not fit |
| `layout-last-not-remainder` | `left` (last child) → `least(minimum(tracks[k], unit, gap), left)` | T2: the frame's last cells are lost |
| `layout-minimum-no-gaps` | `tracks * unit + (tracks - 1) * gap` → `tracks * unit` | `minimum_spec` |
| `layout-share-unweighted` | `tracks * room / weight` → `room / weight` | `share_spec`: a group of two windows would get one window's share (T4) |
| `layout-total-skips-first` | the sum loop starts at `k = 1` | `total_spec` |

Because `tile_spec` states equality with `tileFrom`, any change of behaviour
in `tile` breaks it; the table says which headline fact that change would
have falsified, which is what the comment above each `Mutant(...)` should say.

### 9.3 What is deliberately not proved in Lean

- **The tree.** `Layout` is a recursive type with a `Vec` of itself. The
  kernel today has no `Box`, no `Vec`, no recursion, and a nested recursive
  type through `Vec` is exactly where Aeneas is weakest. A flat pre-order
  encoding with an explicit stack would translate, but its tiling proof needs
  a well-formedness predicate for the encoding and a stack invariant — several
  times the cost of T1–T4 for the same user-visible guarantee. The per-node
  theorem plus the exhaustive-style property test of §8.2 is the 0086 §3
  split: Lean for arithmetic the oracle cannot exhaust, tests for structure.
- **Split and close** (`leaves` ± 1, normal form). Structural, twenty lines
  each, `debug_assert`ed after every change and covered by §8.2 on thousands
  of sequences. If the owner wants these in Lean later, the route is the flat
  encoding above, as its own plan.

## 10. File sizes

| File | Today | After | Note |
|---|---|---|---|
| `editor/panes.rs` | 455 | gone | → `panes/{mod ≈ 260, layout ≈ 230, focus ≈ 120, tests ≈ 420}.rs` |
| `editor/prepare/geometry.rs` | 595 | ≈ 500 | loses `pane_cells` and its tests |
| `editor/prepare.rs` | 677 | ≈ 680 | no growth; if it does grow, split `view_stamp` out in this change |
| `render/buffer.rs` | 376 | ≈ 240 | chrome moves to `buffer/chrome.rs` (≈ 230) |
| `render/golden_tests.rs` | 590 | ≈ 640 | under the ceiling |
| `transaction_conformance/service.rs` | 472 | 472 | two lines change |
| `differential/windows.rs` | — | ≈ 120 | |
| `strop-kernel/src/layout.rs` | — | ≈ 170 | with tests |
| `proofs/Kernel/Layout.lean` | — | ≈ 500–750 | |

`panes/tests.rs` is the one to watch; if it passes ~700, split `geometry`
tests into `panes/tests/geometry.rs` in the same commit.

## 11. Risks

- **Aeneas rejects the shape of `tile`.** Found in the first hour (commit 1
  runs `refresh.sh` first). The pattern — a `while` over a slice writing
  `out[k] = Struct { .. }` with scalar accumulators — is `cache::retire` plus
  `cache::victim`.
- **T4's arithmetic.** Bounded by the fallback rule in §9.1.
- **Pane numbers move.** A split in the middle renumbers the panes after it
  (vim does the same). Stored indexes are audited in commit 2; the tests that
  hard-code "new pane is last" will fail loudly and are listed.
- **Old tapes.** Refused by the seed version, by design.
- **Golden churn.** Existing split goldens must not change (§5.2); a diff
  there is a bug signal, not a re-record.
- **The image's Neovim is newer than the one measured.** Shape rules are
  decades old; focus-after-close is the one to watch (§8.4 says what to do).
- **Tiny terminals.** Below the tree's minimum, later panes shrink to nothing
  (as today with many flat panes). T1 still holds; §8.2 checks no double
  paint. Vim would refuse the split (`E36`); see §12.
- **A terminal pane inside a nest** is resized through its rect like any other
  pane (`prepare_terminal_geometry`); the PTY sees more distinct sizes than
  before. `terminal/pty_tests.rs` covers the split launch; add nothing unless
  a test fails.

## 12. Not in this plan (nice-to-haves, in rough order of value)

1. `ctrl-w t` / `ctrl-w b` (first / last window) — trivial on the tree.
2. `ctrl-w x`, `ctrl-w r` / `R` (exchange, rotate within a split).
3. Resizing: `ctrl-w + - < > _ |`, `:resize`. Needs a stored size per child;
   `tile` would take them as weights. Then `ctrl-w =` does something.
4. `ctrl-w H J K L` (move a window to an edge), `ctrl-w T`.
5. `E36 Not enough room`: refuse a split the canvas cannot hold
   (`minimum(tracks + 1) > cells`).
6. Counts (`3 ctrl-w w`), `:botright` / `:topleft`.
7. `ctrl-w p` exactly as vim after a close (vim forgets a closed previous
   window; strop goes to the most recently used survivor).
8. The layout in the session file.
9. An option for vim's default placement (new window above / left).

## 13. Taste questions for the owner (the implementer takes the default)

1. **Where does the new pane open?** Default: **below / right, focused** — as
   strop does today and as nearly every vimrc sets (`splitbelow splitright`).
   Plain vim opens above / left.
2. **Do `ctrl-w h/j/k/l` (and `Space w h/j/k/l`) wrap at the edge?** Default:
   **no**, silently, as vim. Today they wrap.
3. **Who gets focus when a pane closes?** Default: **vim's rule** — the pane
   that takes the space: the previous neighbour in the same split (its
   top-left pane if it is itself split), else the next. Today: the next
   number.
4. **Sizes.** Default: **always equal by window count, odd cell to the later
   (right / lower) pane**, never depending on which pane had focus. This is
   vim's `equalalways` without `winwidth=20`; sizes can differ from vim by a
   cell (§5.4).
5. **Pane numbers follow the layout** (reading order, vim's `winnr`), so
   splitting pane 1 of 3 makes the new pane number 2. Default: **yes**. The
   alternative, numbers by creation order, makes `Space w N` stable but
   disagrees with what the titles would lead you to expect in a grid.
6. **`┴` under a name.** Default: **the name wins**; the junction is drawn
   only on a rule cell. Inks: `┬ ├ ┤ ┼` `fg.faint` (the divider's); `┴` the
   title's own ink.
7. **`Space t` in a nest.** Default: **split the current pane** (what
   `:split | terminal` does). The alternative is a full-width terminal along
   the bottom (`:botright`), which is item 6 of §12.
