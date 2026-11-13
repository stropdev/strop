# 0103 — Seams and splits: one grammar of lines

- Status: **in force** (0.66, 2026-11-13). Taste question 1 was decided for
  the fallback: every seam is `fg.muted`, wherever the cursor is; focus is the
  name's ink. Every other question took the plan's default.
- Source: the owner — "for the multibuffer edit, is it possible to make the
  different buffers/files more visibly split? I know we have `-----` to split
  files and we aim for consistent UI, but maybe make it like a thick line
  instead? Not sure tbh — give your verdict. Same for the vertical and
  horizontal split UI shape: propose something more consistent with strop's UI
  and polished."
- Builds on [0088](0088-the-look.md) (four greys, one accent, focus is the
  border going to `fg.muted`) and [0100](0100-surfaces.md) (a block is a ruled
  heading; a corner needs a side). Amends 0088 §2 rule 4 (the pane title's
  shape) and 0100 §3.2 / §3.7 (the collection heading's glyph). Adds no colour,
  no plane, no setting.

## 1. What is on screen today

Ground truth is the goldens in `crates/strop/src/render/golden/` (cell grid
plus per-row style runs). Hone inks, measured against `bg.base` `#15171c`:
`fg.faint` `#4a5160` ≈ 2.3:1, `fg.muted` `#858c9a` ≈ 5.3:1.

### 1.1 The collection (`collected-hone-80x24`, `collection-blocks-*`)

```
     search modeline — 5 hits in 2 files

──── docs/remote.md ──────────────────────────────────────────────────── 2 hits
   1 # Remote
   2
   3 The modeline identifies the endpoint.
   4
   5 The modeline names the fixed root.

──── src/render/statusline.rs ────────────────────────────────────────── 3 hits
   1 //! The modeline: one quiet row at the bottom edge.
   2 //!
   3 //! Every width is a display cell, never a character.
   ⋮ 2 lines not shown
   6
```

- **C1 — the file boundary is the quietest thing in the buffer.** In every
  block but the cursor's, the rule is a one-pixel line at 2.3:1 and the name
  drops from `fg.text` to `fg.secondary`. Syntax-coloured code on both sides
  is louder than the line that separates two files.
- **C2 — the levels are upside down.** A jump inside a file (`⋮ 2 lines not
  shown`) is a `bg.surface` band across the whole row. A change of *file* is a
  hairline. The smaller division is drawn with the stronger means. At 48×16
  (`collection-blocks-hone-48x16`) the frame shows two bands and one rule, and
  the bands win.
- **C3 — the boundary matters more here than anywhere.** An edit below the
  line is written to a different file than an edit above it. That is the one
  fact a multi-file buffer must never let you miss.

The shape itself (0100: heading, rows, one blank row; no box; words in the
buffer, lines painted) is right and stays.

### 1.2 Side-by-side panes (`split-hone-80x24`)

```
╭ 1 src/render/statusline.rs ─────────╮ ╭ 2 src/render/statusline.rs ──────────╮
   1 //! The modeline: one quiet row a │   1 //! The modeline: one quiet row at
   2 //!                               │   2 //!
   3 //! Every width is a display cell │   3 //! Every width is a display cell,
~                                      │~
  NORMAL    src/render/statusline.rs                                 45%   9:1
```

- **P1 — a box with no sides.** Each title has two rounded corners and nothing
  under them. This is defect S1 of 0100, which removed exactly this from the
  collection: "a corner needs a side".
- **P2 — the divider does not meet the corners.** `╮` is at column 38, the
  `│` is at column 39, under the blank between `╮` and `╭`. The one vertical
  line on screen starts one row late and one cell off from the shapes above it.

### 1.3 Stacked panes (`terminal-hone-80x24`)

```
╭ 1 src/render/statusline.rs ──────────────────────────────────────────────────╮
   9 │   chip: String,
  10 │   dirty: bool,
╰──────────────────────────────────────────────────────────────────────────────╯
╭ 2 term:///work//1:sh ────────────────────────────────────────────────────────╮
$ cargo test
```

- **P3 — two rules touch.** `╰──╯` sits directly on `╭──╮`: 0100's S2 ("the
  eye sees one double rule, not an end and a start"). One boundary costs two
  rows.
- **P4 — only some panes close.** The upper pane has a bottom border; the
  lowest pane has none (the modeline follows). In a row layout no pane has one.
  Three different answers to "where does a pane end".

So the panes still carry the two defects 0100 diagnosed and fixed in the
collection. They were not in 0100's scope; this plan finishes the job.

## 2. How others draw it

- **Zed multibuffer / VS Code search editor / GitHub diff.** A file starts with
  a filled header bar holding the path; jumps inside the file are thin
  separators. *Take:* the file boundary is the strongest mark in the buffer,
  the jump the weaker one. *Leave:* the filled bar — strop's band already means
  "jump" (0100 §3.4), and sticky headers and fold buttons are widgets.
- **Neovim quickfix + bqf, `grug-far`, Helix global search.** File name as a
  text row, sometimes highlighted; no geometry at all. Works, but relies on
  colour alone — strop's ruled heading is already better.
- **Rich / Textual tables.** Heavy rule under the header, light rules in the
  body. The best-known case of two line weights reading as polished: the
  weight has exactly one meaning.
- **tmux, zellij, Helix, Neovim splits.** One continuous divider line; tmux and
  zellij join lines with `┬ ├ ┼`; Neovim's `winbar`/`laststatus=3` setups draw
  a title row per window and a `│` column. Nobody draws corners without sides.
  *Take:* continuous lines that meet in junctions. *Leave:* zellij's full frame
  around every pane (two columns and two rows per pane).

## 3. Verdict

**Yes to the thick line for file boundaries — with one job and nothing else.**
The instinct is right: C2 says the file boundary must outrank the band, and a
heavy rule is the only *rule* that does. Raising the ink instead (every rule
`fg.muted`) was considered and is weaker: today's cursor block already looks
like that, it spends the focus signal of 0088 rule 3, and it still leaves a
hairline competing with a filled band.

The heavy line is safe only if it is scarce. It gets the accent's discipline
(0088 rule 2): **one weight, one job — a heavy rule is a seam: the place in one
buffer where the rows start belonging to another file.** Only the collection
has seams. Status sections, the diff's file heading, the commit-files heading,
help sections and the picker's group header are not seams and stay light.

**Splits: drop the corners, keep the rule, join the lines.** A pane starts at
a ruled title — the same grammar as every heading — and ends at the next rule
or the modeline. The vertical divider runs the full height and meets the title
rules in a `┬`. The closing `╰──╯` row is deleted and its row goes back to
text. This is 0100 §3.3 applied to panes; nothing new is invented.

One sentence for the whole system: **light lines are the frame between views;
a heavy line is a seam inside a buffer; a band is a jump inside a file.**

## 4. The grammar

### 4.1 Glyphs

| Glyph | Name | Means | Where |
|---|---|---|---|
| `─` | light rule | a view or a block starts here | pane titles; headings of status, diff, commit files, help; leaders |
| `│` | light bar | two views side by side | pane divider (also, unchanged: the diff sidebar, the status rail, card sides) |
| `┬` | junction | the divider starts at the title row | the divider column, title row only |
| `━` | heavy rule | **seam**: another file's rows start here | collection file headings, gutter rule and leader both |
| band (`bg.surface`) + `⋮` | break | a jump inside one file | unchanged (0100 §3.1) |
| `╭ ╮ ╰ ╯` | corners | a card: a box that has all four sides | cards only (picker, which-key, completion, hover). **Never on panes.** |

*Junctions drawn by [0104](0104-window-tree.md).* Reserved for the tree layout (the follow-up named in
`strop-engine/src/editor/panes.rs`; not built here): `├ ┤ ┴ ┼` where a stacked
title meets a divider. Because titles are rules, those junctions fall out with
no new idea.

All of these are in the Unicode Box Drawing block strop already depends on
(`─ │ ╭`); terminals that draw box glyphs themselves (Windows Terminal, kitty,
WezTerm, Alacritty, foot) draw `━` and `┬` the same way. 0076 sets no glyph
constraint. No ASCII fallback is added (there is none for `╭` either).

### 4.2 Inks

| Element | Focused | Not focused |
|---|---|---|
| pane title rule | `fg.muted` | `fg.faint` |
| pane title number, name | as today (`fg.text`; dir `fg.secondary`, name bold) | as today (`fg.muted`, name bold) |
| pane divider `│` and `┬` | `fg.faint` always — it belongs to neither pane | |
| seam `━` (gutter rule + leader) | cursor's block: `fg.muted` | other blocks: `fg.faint` |
| seam name, marks, fact | as today (0100 §3.2) | as today |
| every other heading | unchanged | unchanged |

No ink changes at all. The seam gets heavier by glyph; the panes get cleaner
by shape. `focused` for a pane means the active pane; for a seam, the block the
cursor is in — both are 0088 rule 3, unchanged.

### 4.3 Who owns a line

- A title rule belongs to the pane **below** it and takes that pane's focus ink.
- The divider column belongs to no pane.
- A seam belongs to the file **below** it.

So a line never changes colour because of what is above it. (Today the closing
`╰──╯` takes the colour of the pane above, the title the colour of the pane
below, and they touch.)

## 5. Targets

Trailing blanks are not shown. Column 79 at 80 columns is the scrollbar's.

### 5.1 (a) A collection — before

```
     search modeline — 5 hits in 2 files

──── docs/remote.md ● ────────────────────────────────────────────────── 2 hits
   1 # Remote
   2
   3 The modeline dentifies the endpoint.
   4
   5 The modeline names the fixed root.

──── src/render/statusline.rs ────────────────────────────────────────── 3 hits
   1 //! The modeline: one quiet row at the bottom edge.
   2 //!
   3 //! Every width is a display cell, never a character.
   ⋮ 2 lines not shown
   6
   7 /// Gather the row.
   8 pub struct Modeline {
```

### 5.2 (a) A collection — after

```
     search modeline — 5 hits in 2 files

━━━━ docs/remote.md ● ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 2 hits
   1 # Remote
   2
   3 The modeline dentifies the endpoint.
   4
   5 The modeline names the fixed root.

━━━━ src/render/statusline.rs ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 3 hits
   1 //! The modeline: one quiet row at the bottom edge.
   2 //!
   3 //! Every width is a display cell, never a character.
   ⋮ 2 lines not shown
   6
   7 /// Gather the row.
   8 pub struct Modeline {
```

- Cursor on line 3 of `docs/remote.md`: that seam is `fg.muted`, the other
  `fg.faint`. Everything else — title, name inks, `●`, fact, band, `⋮`, blank
  row between blocks, cursor row — is byte-for-byte today's.
- Same cells, same rows, same layout table (0100 §3.9). Only two glyphs change.

Narrow (the heading's part table is untouched, so the cuts are today's):

```
━━━━ src/b.rs ● ━━━━━━━━━━━━━━━━━━━━━━━━ 2 hits     48
━━━━ b.rs ● ━━━━ 2 hits                             24
━━━━ a_very_long_modu…  10 hits                     32   (leader gone: plain gap)
```

### 5.3 (b) Side by side — before / after

Before: §1.2. After, pane 1 active:

```
─ 1 src/render/statusline.rs ──────────┬─ 2 src/render/statusline.rs ───────────
   1 //! The modeline: one quiet row a │   1 //! The modeline: one quiet row at
   2 //!                               │   2 //!
   3 //! Every width is a display cell │   3 //! Every width is a display cell,
   4                                   │   4
   5 use std::path::Path;              │   5 use std::path::Path;
~                                      │~
  NORMAL    src/render/statusline.rs                                 45%   9:1
```

- Columns 0–38: pane 1's title, rule `fg.muted`, `1` and the path in the
  active inks. Column 39: `┬` then `│` down to the row above the modeline,
  `fg.faint`. Columns 40–79: pane 2's title, rule `fg.faint`, text `fg.muted`
  with the name bold.
- With pane 2 active only the inks swap; no cell changes.
- Same geometry as today: the title is `─ N name flags ` + rule to the pane's
  last cell. `╭` became `─`, the closing `─╮` became `──`, the blank over the
  divider became `┬`.

Three panes:

```
─ 1 src/a.rs ─────────────┬─ 2 src/b.rs ● ───────────┬─ 3 docs/remote.md ───────
   1 //! The modeline: one│   1 //! The modeline: one│   1 //! The modeline: one
```

### 5.4 (c) Stacked — before / after

Before: §1.3. After, pane 1 active, 80×24:

```
─ 1 src/render/statusline.rs ───────────────────────────────────────────────────
   1 //! The modeline: one quiet row at the bottom edge.                       ▮
   …
  10 │   dirty: bool,
─ 2 term:///work//1:sh ─────────────────────────────────────────────────────────
$ cargo test
   Compiling demo v0.1.0
   …
 TERMINAL   term:///work//1:sh                      ctrl-\ ctrl-n  NORMAL  1:1
```

- Row 0: pane 1's title, `fg.muted`. Rows 1–10: its text. Row 11: pane 2's
  title, `fg.faint` — it is also the boundary. Rows 12–22: pane 2 (11 rows;
  today 10). Row 23: the modeline.
- One boundary costs one row. `n` stacked panes get back `n − 1` rows.
- Every pane ends the same way: at the next rule, or at the modeline.

### 5.5 (d) Mixed: a file beside a collection, collection active

Layouts are flat today (a row or a column), so "mixed" is a collection in a
split. 100 columns:

```
─ 1 src/render/statusline.rs ────────────────────┬─ 2 collection: search modeline ● ────────────────
   1 //! The modeline: one quiet row at the botto│     search modeline — 5 hits in 2 files
   2 //!                                         │
   3 //! Every width is a display cell, never a c│━━━━ docs/remote.md ● ━━━━━━━━━━━━━━━━━━━━ 2 hits
   4                                             │   1 # Remote
   5 use std::path::Path;                        │   2
   6                                             │   3 The modeline dentifies the endpoint.
   7 /// Gather the row.                         │   4
   8 pub struct Modeline {                       │   5 The modeline names the fixed root.
   9     chip: String,                           │
  10     dirty: bool,                            │━━━━ src/render/statusline.rs ━━━━━━━━━━━━ 3 hits
  11     line: usize,                            │   1 //! The modeline: one quiet row at the bottom
  12 }                                           │   2 //!
```

- Three levels, three means, visible at once: the light frame (title rules,
  `┬`, `│`), the heavy seams, and (further down) the bands.
- A seam touches the divider (`│━━━━`) and does not join it: the seam is a row
  of the buffer, the divider is the frame. The weights keep them apart; today's
  `│────` reads as a `├` that is not there.
- Pane 1 inactive: title rule `fg.faint`, text `fg.muted`. Pane 2 active: title
  rule `fg.muted`. Inside pane 2 the seam of the cursor's block is `fg.muted`,
  the other `fg.faint`. In an **inactive** pane showing a collection the seams
  keep whatever block that pane's saved cursor is in (the renderer already
  computes `block_active` per pane; unchanged).

## 6. Edge cases

- **One pane.** No title, no divider (0088 rule 4, unchanged). A collection's
  first seam is then the first line on screen after the buffer title.
- **Narrow panes.** The title's arithmetic is today's: number, `●` and `[RO]`
  are reserved before the name clips; the trailing rule is drawn only while the
  whole name fits (`fixed + name + 3 <= width`). The `┬` is in the divider
  column and is always drawn, so the divider is whole even when a title has no
  room for its rule: `─ 1 sample.rs ● [RO]┬─ 2 sample.rs ● [RO]`.
- **Zero-size panes** (terminal shorter or narrower than the panes need):
  `pane_cells` already clamps; a pane with no height draws no title. Unchanged.
- **Scrollbar.** The pane's last column stays reserved for the thumb below the
  title; the title rule runs through it, as the border does today.
- **A collection with one file.** One seam. Still heavy: the rule says "these
  rows are this file's", not "there are two files".
- **Empty collection / loading.** No heading rows, so no seams. Unchanged.
- **No folding exists** in the collection; nothing to specify.
- **Sideways scroll.** Headings are `fixed` rows (laid out against the pane);
  the seam does not scroll. Unchanged.
- **Cursor on a seam.** `bg.cursorline` across the row over the heavy rule,
  caret at the text column on the path's first byte. Unchanged.
- **Yank, search, motions never see decoration.** The heading's buffer text is
  the path (0100 §3.8); `━` and `┬` are painted. The two assertions in
  `strop-engine/src/editor/collections/tests/mod.rs` (lines 479 and 512) gain
  `'━'` and `'┬'` in their forbidden sets.
- **Light themes.** `fg.faint` on Vellum is as quiet as on Hone by the theme
  test's rules; the heavy glyph is what carries the seam in both. See taste
  question 1.
- **Terminal panes.** A terminal's title is `─ 2 term:///work//1:sh ───`; its
  PTY size comes from the prepared rect, so the lower pane of a stacked pair
  grows by one row and the child sees one resize on upgrade. Nothing special.

## 7. Considered and rejected

- **A filled header band for the file** (Zed, VS Code, GitHub). The band is
  0100's "jump"; two bands on two planes one ladder step apart (1.07–1.30:1)
  would be told apart by nothing a tired eye can see. Swapping the two (band =
  file, rule = jump) would redraw every git surface for no gain.
- **A background tint per file.** A new plane under code that already carries
  the cursor line, selection, search and diff tints. No.
- **Raising the ink only** (all rules `fg.muted`, names always `fg.text`). The
  cheapest change, and not enough: a hairline still loses to a band, and the
  cursor block loses its signal. Kept as the fallback in taste question 1.
- **Double line `═`, block glyphs `▀▄`.** Read as DOS and as a progress bar.
- **A full-width rule on the blank row above the heading.** Stronger, but the
  block loses its breathing row and the path loses the line it sits on.
- **Heavy rules on every heading** (status sections, diff, commit files, help,
  the picker). Then heavy means "heading", light means "leader", and the status
  buffer gets louder for nothing. Scarcity is the meaning.
- **Heavy divider `┃` or heavy title for the active pane.** A second job for
  the heavy weight. Focus is ink (0088 rule 3).
- **A real frame around each pane** (zellij). Two columns and a row per pane;
  0088 rule 4 forbids chrome that carries nothing.
- **Keeping the corners and moving the `│` under them.** Still a box with a top
  and one side; in a column layout still two touching rules.
- **Sticky file header while scrolling a long block.** A widget over a buffer
  (doctrine 4). The modeline could name the cursor's file instead — out of
  scope, see question 6.

## 8. Implementation

Two independent changes; land them as two commits, seams first (small), panes
second (touches geometry).

### 8.1 Seams

`crates/strop/src/render/block/heading.rs`

- Replace `Heading.ruled: bool` with a typed rule (data, not a flag pair):

  ```rust
  /// The line a heading is set into (0103 §4.1).
  pub(crate) enum Rule { None, Light, Seam }
  ```

  with one `fn glyph(self) -> &'static str` (`"─"` for `None` and `Light`,
  `"━"` for `Seam`). `None` keeps a blank gutter and the light leader — the
  picker's group header, unchanged.
- `Heading::draw`: the gutter rule (`"─".repeat(gutter_cells - 1)`) and the
  leader (`"─".repeat(gap - 1 - trailing)`) both take `self.rule.glyph()`.
  The part table, `layout`, sizes and inks do not change.
- Module doc: one sentence naming the seam and this plan.

Callers (every `Heading { .. ruled: .. }`; migrate all, no alias):

| File | Was | Becomes |
|---|---|---|
| `render/buffer/rows/collection.rs` (`CollectionRow::Heading`) | `ruled: true` | `Rule::Seam` |
| `render/diff.rs`, `render/diff/status.rs`, `render/diff/files.rs` (×2) | `ruled: true` | `Rule::Light` |
| `render/picker/rows/search.rs` | `ruled: false` | `Rule::None` |

Tests:

- `render/block/heading/tests.rs`: the table at lines 43–48 keeps its light
  cases (they pin the part table); add the same strings for `Rule::Seam` with
  `━` at 119, 79, 47, 31 and 23 cells, proving the two weights lay out
  identically.
- `render/block_frame_tests.rs` (`Kind::Heading`, line ~118): the rule's first
  cell is `━` in a collection and `─` on every other surface; the "headings of
  a block end together" check is glyph-agnostic already.
- `render/buffer/tests/collection.rs`: any heading string pinned with `─`.
- `strop-engine/src/editor/collections/tests/mod.rs` lines 479, 512: add `'━'`.
- Unchanged and must stay green as proof the scope held:
  `render/buffer/tests/diagnostics.rs:162`, `render/buffer/tests/viewport.rs:403`,
  `headless.rs:136` (diff headings, light), `picker/rows/tests.rs` (light leader).

Goldens rewritten (`STROP_UPDATE_GOLDEN=1 cargo test`; review the diff — only
heading rows may change, and only `─` → `━`): `collection-*`, `collected-*`,
`collection-blocks-*` (4 sizes × 2 themes). No git golden may change.

### 8.2 Panes

`crates/strop-engine/src/editor/prepare/geometry.rs` — `pane_cells`

- The gap between panes exists only side by side: `usable = axis − (count − 1)`
  in a row layout, `usable = axis` in a column layout; `offset` advances by
  `size + 1` in a row layout and by `size` in a column layout. The title row
  subtraction (`count > 1`) is unchanged.
- Doc comment: a stacked pane's title row is its boundary; there is no
  closing row.
- Tests beside it (or in `prepare`'s tests): 2 stacked panes in 80×24 are
  `(y 1, h 10)` and `(y 12, h 11)`; 3 stacked in 80×24 use all 23 rows; row
  layout rects are exactly today's; heights never overlap and never pass the
  modeline at 1×1 … 200×60 (a small property loop).

`crates/strop/src/render/buffer.rs`

- `render_title_border` → `render_title_rule`. `"╭ "` → `"─ "`. The closing
  span becomes a space and `─` to the pane's last cell (today: space, rule,
  `╮`); `closes` and the budget arithmetic keep their numbers (the 3 reserved
  cells are now ` ──`).
- `render_divider`: only the row layout has one. Title-row cell (`dy + 1 ==
  rect.y`) is `┬`, the rest `│`, all `fg.faint` on `bg.base`. Delete the
  column branch (`╰ ─ ╯`), the `active` parameter and the "pane it belongs to
  decides the colour" comment.
- `render_panes`: `axis`, `size` and the divider call follow `pane_cells` —
  no `h + usize::from(n > 1)` for a column layout, no divider call when
  stacked. Prefer deriving the divider column from the prepared rects
  (`rect.x + rect.width`) over recomputing an offset, so paint cannot disagree
  with preparation.
- Module doc ("Chrome (0088 §2 rule 4)") rewritten to §4 of this plan.

Tests:

- `render/buffer/tests/identity.rs`: `narrow_titles_preserve_focus_flags_
  geometry_and_single_vertical_rule` — `╭ 2 ` becomes `─ 2 `; the cell left of
  the second title's `─` on row 0 is `┬` and the same column on row 1 is `│`;
  the stacked branch asserts **no** `╰` anywhere and that a row starting `─ 2 `
  exists. Line ~150 `titles.contains("╭ 2 [scratch]")` → `─ 2 [scratch]`. Add:
  no cell of a pane frame is `╭ ╮ ╰ ╯` in either layout at 80×24, 48×16, 24×10.
- New, same file: active/inactive inks — title rule cell is `t.muted` for the
  active pane and `t.faint` for the other, `┬` and `│` are `t.faint`, in both
  layouts, and swapping focus changes no symbol.
- `render/buffer/tests/viewport.rs`: any pane-title strings.
- `strop-engine/src/editor/terminal/pty_tests.rs`: if it pins the stacked
  terminal's row count, it grows by one.
- e2e / headless journeys that grep frames for `╭ 1` or `╰` (search
  `crates/` and `tests/` for `"╭ "` and `'╰'` outside cards).

Goldens rewritten: `split-*`, `terminal-*`, `terminal-normal-*`. Added (in
`render/golden_tests.rs`, the `each` sizes): `split-stacked` (two file panes
stacked, the **lower** active, so the inactive title on row 0 and the active
one mid-screen are both pinned), `split-three` (three side by side, the middle
active), `split-collection` (§5.5: a file beside a collection, collection
active).

### 8.3 Words

- `docs/editor.md` (pane title paragraph near line 144) and any frame in
  `docs/`; `CHANGELOG.md` entry in plain words ("file boundaries in a
  collection are a thick line; panes lose their corners, stacked panes get a
  row back").
- `plans/STATUS.md`: a row for 0103. `plans/0088` rule 4 and `plans/0100`
  §3.2 get a one-line "amended by 0103" pointer.
- The site's frames are captured from the release binary (`tools/frames.py`);
  recapture after the release, do not hand-edit.

### 8.4 Gate

`docker compose run --build --rm test`. No kernel code moves; `proofs/` is
untouched. Neither file nears the size ceiling (`buffer.rs` shrinks).

## 9. Taste questions for the owner

1. **Is the seam strong enough at `fg.faint`?** The plan keeps today's inks and
   lets the glyph do the work. If other files' seams still read too quiet in a
   real terminal (check Vellum too), the fallback is one line: seams are
   `fg.muted` everywhere and the cursor's block keeps its bold `fg.text` name
   as the only focus signal.
2. **Should names in other blocks stop dimming?** Today a file name is
   `fg.text` bold in the cursor's block and `fg.secondary` bold elsewhere. With
   a heavy seam the dimming may no longer be wanted — you scan the *other*
   headings to decide where to go.
3. **Heavy only for the collection — agreed?** The alternative is "every
   buffer heading is heavy" (status sections, the diff's file heading, commit
   files). The plan says no: one weight, one job.
4. **The picker's Search group header stays light** (`docs/remote.md ──── 2`),
   so the same file is a light leader in the picker and a heavy seam after
   `Ctrl-O`. Acceptable (a list in a card is not a buffer of stitched files),
   or should the picker's leader match?
5. **Losing the rounded corners on panes.** They are the one place the pane
   echoed the cards. The plan says corners are for boxes with four sides. If
   you miss the softness, the only honest way back is a full frame, which costs
   two columns per pane.
6. **`┬` or a plain gap over the divider?** The junction makes the frame one
   drawing; a blank cell is quieter but repeats P2 in a milder form.
7. **Stacked panes end at the modeline with no rule of their own**, and the
   lower pane gets the extra row. Fine, or should the freed row stay empty
   above the lower title as breathing room?
8. **Not in this plan, worth a yes/no for later:** the modeline naming the
   cursor's file inside a collection (`collection: search modeline › src/b.rs`)
   for when the seam has scrolled away; and the diff view's sidebar divider
   meeting its heading (`│────` → `├────`).
