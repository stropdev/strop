# 0105 — The number gap: two cells between a line number and its text

- Status: **proposed** (2026-11-15). Not implemented. The plan is complete
  with the recommended answer to every taste question in §9, so work can start
  without waiting.
- Source: the owner — "tweak the spacing between the text and the line number
  column … to improve the look of strop."
- Builds on [0088](0088-the-look.md) (four greys, one accent, chrome only when
  it carries information) and [0100](0100-surfaces.md) §3.5 (the gutter is
  data, chosen in one place). Amends 0100 §3.5 (the cell counts of the lines,
  source and pair gutters) and the picker preview's gutter. Adds no colour, no
  glyph, no setting.

## 1. What is on screen today

Ground truth is the goldens in `crates/strop/src/render/golden/` and
`render/block/gutter.rs`. A file buffer's gutter is `sign + digits + 1`, with
at least three digits (`prepare/geometry.rs`, `number_gutter`):

```
          1         2         3
0123456789012345678901234567890123456789
   7 /// Gather the row.
   8 pub struct Modeline {
   9 │   chip: String,
▎ 10 │   dirty: bool,
● 11 │   line: usize,
  12 }
```

| Cells | Holds | Ink |
|---|---|---|
| 0 | sign: `●` diagnostic (wins), `▎` git hunk, `▌` folder cursor, else blank. Always reserved. | severity / diff inks, bold |
| 1–3 | line number, right-aligned, absolute | `fg.muted`; the cursor line `accent` |
| 4 | one blank | — |
| 5… | text | — |

Facts found while reading the code, so nobody plans around things that do not
exist: strop has **no relative numbers, no soft wrap (long lines scroll
sideways), no fold column and no mouse input** (`terminal/decode.rs:210` drops
mouse reports). Diagnostics and git share the one sign column.

Defects:

- **N1 — the number touches the text.** One blank cell is the same distance
  as the space between two words, so `10 │` and `8 pub` read as part of the
  line. The eye has to find where the code starts on every row.
- **N2 — in Kiln the number and a comment are the same ink.** Numbers are
  `fg.muted` `#928d83`; comments are `#928c81` italic. `   1 //! The modeline`
  is one grey run. Only distance can separate them: there are four greys
  (0088 rule 8) and `fg.faint` is about 2.3:1, too dim for a number you read
  for `:42` and `12G`.
- **N3 — the first indent guide looks like a broken separator.** A guide at
  text column 0 sits one cell after the number (`  9 │   chip`). It reads as a
  gutter bar that exists on indented rows and is missing on the others.
- **N4 — three answers to the same question.** The Search picker's hit rows
  already put two cells after the number (`▌    3  The modeline…`,
  `picker/rows/search.rs:92`). The picker preview puts ` │ ` there
  (`picker/preview.rs:189`). The buffer puts one cell.

The sign column, the right alignment, the three-digit floor and the inks are
right and stay.

## 2. How others space it

- **Neovim.** `numberwidth` defaults to 4 and includes the blank: three digits
  and **one** cell. `signcolumn=auto` adds two cells left of the number only
  while a sign exists, so the text jumps sideways when the first diagnostic
  arrives; most configurations pin it with `signcolumn=yes`. *Take:* a reserved
  sign column, a three-digit floor. *Leave:* the one-cell gap — it is the look
  people fix first with `statuscolumn`.
- **Helix.** Default gutters are `diagnostics, spacer, line-numbers, spacer,
  diff`: number, a spacer, the diff column, then text — **two** cells between
  number and text, one of them usually empty. No bar. *Take:* two cells.
  *Leave:* a second sign column between number and text; strop's signs live at
  column 0 in every gutter and in every list (0088 rule 5).
- **VS Code, Zed.** Number, then a strip of roughly two to three characters
  that holds the change bar, the fold arrow and code-action marks, then text.
  No vertical line. The strip is wide because it holds widgets. *Take:* the
  number column is set off by space, never by a line. *Leave:* the width —
  strop has nothing to put there.
- **A terminal grid** has no half cells. One cell is word spacing; two cells
  is the smallest distance that reads as a column gap (it is what `ls -l`,
  `git log --graph` tables and strop's own lists use between columns); three
  starts to look like indentation.

## 3. Verdict

**Two blank cells between the number and the text, everywhere a number sits
left of text. Nothing else moves.** One constant, `NUMBER_GAP = 2`, read by
the engine's geometry and the renderer.

- No separator bar. A `│` there would say nothing the gap does not (0088
  rule 4), would collide with the pane divider one cell away in the preview,
  and would look like an indent guide.
- No padding left of the sign column. The sign column is the left margin when
  it is empty; a second margin costs a cell and says nothing.
- The sign column stays reserved when empty, so text never shifts when a
  diagnostic or a hunk appears.
- The picker preview drops its ` │ ` and takes the buffer's gutter, so a
  preview row is the row the buffer will draw after `Enter`.
- The cost is one text cell per pane (two saved in the preview for files of
  100 lines or more).

## 4. The layout

### 4.1 Cells

| Gutter | Before | After |
|---|---|---|
| lines (file, help, folder, review) | `sign + digits + 1` | `sign + digits + 2` |
| source (collection) | `sign + digits + 1` | `sign + digits + 2` |
| pair (diff, hunk) | `sign + old + 1 + new + 1` | `sign + old + 1 + new + 2` |
| list (status, log, commit files) | `sign + 1` | unchanged — no number, no number gap |
| picker preview | `mark + 1 + digits + " │ "` | `mark + digits + 2`, the lines gutter |

`digits` is the width of the largest number shown, at least 3 (unchanged). The
preview takes the same floor, so it has the buffer's text column.

Number: right-aligned, absolute. Left of the number: the sign column and the
unused digit cells, nothing more.

### 4.2 Inks (no change, stated so the gap is defined)

| Part | Ink |
|---|---|
| sign | `●` severity ink, `▎` `diff.add` / `diff.change` / `diff.del` (staged-only: dimmed add), `▌` `accent`; bold |
| number | `fg.muted`; cursor line `accent` (0088 rule 2) |
| gap | blank; background is the row's (`bg.base`, or `bg.cursorline` on the cursor line, which covers the whole gutter) |
| preview number | `fg.muted` on `bg.surface`; focused line `fg.text` bold on `bg.raised` with `▌` in `accent` (unchanged) |
| `~` past the end | `fg.faint`, column 0 (unchanged) |

### 4.3 What follows without a decision

- A collection's seam and a diff's heading rule run `gutter − 1` cells, then
  one blank, then the name at the text column (0100 §3.2). They grow by one
  cell with the gutter.
- `⋮` on a break stays in the last digit cell of each number column.
- The blame column (22 cells) and the commit sidebar sit left of the gutter
  and do not change.

## 5. Targets

Rulers count cells from the pane's first column.

### 5.1 A file at 80 columns, a git sign and a diagnostic

```
          1         2         3         4         5         6         7
01234567890123456789012345678901234567890123456789012345678901234567890123456789
before
   7 /// Gather the row.
   8 pub struct Modeline {
   9 │   chip: String,
▎ 10 │   dirty: bool,
● 11 │   line: usize,
  12 }
after
   7  /// Gather the row.
   8  pub struct Modeline {
   9  │   chip: String,
▎ 10  │   dirty: bool,
● 11  │   line: usize,
  12  }
```

Sign 0, number 1–3, gap 4–5, text from 6, scrollbar column 79. Text budget
74 → 73 cells.

### 5.2 The cursor line

```
0123456789012345
▎ 10  │   dirty: bool,
```

`10` in `accent` (`#dc7a59` in Kiln); cells 0–79 on `bg.cursorline`, gap
included; the caret for buffer column 0 is on cell 6.

### 5.3 A long line (there is no wrap)

strop has no soft wrap, so there is no continuation row to draw. A long line
scrolls sideways and the gutter stays fixed:

```
0123456789012345
before, hscroll 38
   1 xxabZ
after, hscroll 38
   1  xxabZ
```

If wrap is ever added, the rule is already decided by this plan: a
continuation row has a blank gutter of the full width and its text starts at
the text column.

### 5.4 A narrow pane (three panes at 80 columns, each 26 cells)

```
          1         2
012345678901234567890123456
before
   1 //! The modeline: on │
   9 │   chip: String,    │
after
   1  //! The modeline: o │
   9  │   chip: String,   │
```

The gap does not shrink in a narrow pane (§7).

### 5.5 The picker preview (`grep-hone-80x24`, `locations-hone-80x24`)

```
0123456789012345678901234567890
before, a 5-line file
│  1 │ # Remote                │
│  2 │                         │
│▌ 3 │ The modeline identifies │
after
│   1  # Remote                │
│   2                          │
│▌  3  The modeline identifies │

before, a 20-line file
│   2 │ //!                  │
│▌  3 │ //! Every width is a │
after
│   2  //!                   │
│▌  3  //! Every width is a  │
```

Text column: same for files under 10 lines, one cell earlier for 10–999
lines, two earlier from 1000.

### 5.6 Four digits

```
0123456789012345
before
  999 x
▎1000 x
after
  999  x
▎1000  x
```

Gutter 6 → 7 cells. (Both rows are from a 1001-line file; the gutter is as
wide as the largest number, on every row.)

### 5.7 A diff and a collection

```
0123456789012345678901234567890
before
──────── src/lib.rs ─────────
   ⋮   ⋮ @@ -1,5 +1,5 @@
   1   1     let value_1 = compute(1);
▎  2         let value_2 = compute(2);
▎      2     let second = compute(2);
after
───────── src/lib.rs ────────
   ⋮   ⋮  @@ -1,5 +1,5 @@
   1   1      let value_1 = compute(1);
▎  2          let value_2 = compute(2);
▎      2      let second = compute(2);

before
━━━━ docs/remote.md ━━━━━━━━━
   1 # Remote
   ⋮ 2 lines not shown
after
━━━━━ docs/remote.md ━━━━━━━━
   1  # Remote
   ⋮  2 lines not shown
```

## 6. Edge cases

- **A pane narrower than the gutter.** Unchanged rule: the text width is zero,
  the gutter is cut at the pane edge, the horizontal origin is kept
  (`narrower_than_the_gutter_keeps_the_margin_fixed`).
- **A file that grows from 999 to 1000 lines.** The gutter widens by one cell,
  as today.
- **Help, folder, review.** They use the lines gutter and move with it. The
  folder's size and age columns are laid out in the text width and keep their
  alignment.
- **The terminal pane in Normal mode** shows the lines gutter and moves too.
- **Preview narrower than its gutter.** Three tiers stay: full gutter;
  `mark + digits + 1` when the full one does not fit; the mark alone.
- **The Search picker's hit rows** already have the two-cell gap and do not
  change.

## 7. Considered and rejected

- **A `│` or `▏` between number and text.** No information; a third vertical
  line next to the divider and the indent guides.
- **Three cells.** Reads as indentation of the whole file and costs two cells
  per pane in a three-way split.
- **Numbers in `fg.faint`.** About 2.3:1 on every palette; numbers are read,
  not just seen.
- **A fifth grey for numbers.** Breaks 0088 rule 8 for a problem that distance
  solves.
- **The git sign between number and text (Helix).** It would give the gap a
  job, but strop's sign is at column 0 in every gutter and list; two sign
  positions is two vocabularies.
- **A gap that shrinks in narrow panes.** The text column would move when a
  split is resized. One constant.
- **Left padding before the sign.** One more cell for nothing.
- **A setting** (`numberwidth`, `gutter.gap`). Zero-config; one look.
- **Dropping the three-digit floor.** The gutter would change width at 10 and
  100 lines; small files would differ from big ones.

## 8. Implementation

Every caret, clamp and paint reads the gutter width from `Editor::gutter` /
`Editor::left_inset` (`prepare/geometry.rs:415`, `:443`). Those callers need
**no change** and must not grow a second source: `render/buffer.rs:142`
(`caret_position`, used by `render/mod.rs:160` and the inactive-pane caret),
`editor/panes/focus.rs:106` (the frame's cursor), `editor/prepare.rs:281`
(horizontal reveal), `render/buffer/rows/mod.rs:97-101` (paint and indent
guides), popups anchored on the caret.

### 8.1 Engine

| Site | Change |
|---|---|
| `crates/strop-engine/src/editor/prepare/geometry.rs:30-32` | delete `STANDARD_GUTTER` (its only reader is one test) |
| same file, near `:402` | add `pub const NUMBER_GAP: usize = 2;` — "blank cells between the last number column and the text" |
| same file `:405-409` | `number_gutter` becomes `pub`: `1 + digits.max(3) + NUMBER_GAP` |
| same file `:390-399` | `Gutter` variant docs: `+ 1` → `+ the number gap` |
| `crates/strop-engine/src/editor/prepare.rs:33-38` | re-export `NUMBER_GAP`, `number_gutter`; drop `STANDARD_GUTTER` |
| `crates/strop-engine/src/editor/git_memory/presentation.rs:55` | `gutter_width: 1 + digits + 1 + digits + NUMBER_GAP` |

### 8.2 Renderer: `crates/strop/src/render/block/gutter.rs`

| Site | Change |
|---|---|
| `numbered` `:33-47` | `digits = width − 1 − NUMBER_GAP`; the cell is the number then `NUMBER_GAP` blanks; `None` is `digits + NUMBER_GAP` blanks |
| `source_jump` `:71-73` | `width − NUMBER_GAP − 1` |
| `pair_digits` `:82-84` | `(width − 2 − NUMBER_GAP) / 2` |
| `pair` `:89-114` | the last span is `NUMBER_GAP` blanks |
| doc comments `:30-32`, `:70`, `:75` | say "the number gap", not "a space" |
| test `every_gutter_paints_its_width` `:126-160` | largest number `10^(cells − 1 − NUMBER_GAP) − 1`; pair cells `2·digits + 2 + NUMBER_GAP`; the `⋮` string for a 10-cell pair gutter is `"   ⋮   ⋮  "` |

`render/block/heading.rs:316` (rule = `gutter − 1`, one blank) and
`render/block/mod.rs` need no change.

### 8.3 Renderer: `crates/strop/src/render/picker/preview.rs`

`highlight_lines_owned` `:154-197`: the full gutter is
`number_gutter(preview_source_lines(rope))`; its row is the mark, the number
right-aligned in `gutter − 1 − NUMBER_GAP` cells, `NUMBER_GAP` blanks. The
` │ ` span (`:189`) is deleted. The second tier (`mark + digits + 1`) and the
third (mark only) keep their shape; the tier test compares against the new
full width. Inks do not change.

### 8.4 Tests that change

Hard-coded columns; each moves by the gap and should read the engine instead
of a literal where it can (`editor.left_inset(doc)`):

| Site | Today | Becomes |
|---|---|---|
| `render/buffer/tests/diagnostics.rs:58` | `STANDARD_GUTTER` | `e.left_inset(e.current())` |
| `render/buffer/tests/diagnostics.rs:104-109` | selection cells `[8, 9, 10, 11]` | `[9, 10, 11, 12]` |
| `render/buffer/tests/diagnostics.rs:113-124` | 7 cells `" 1001 x"`, caret `(6, 2)` | 8 cells `" 1001  x"`, caret `(7, 2)` |
| `render/buffer/tests/diagnostics.rs:150-165` | 13-column terminal, 9 fixed cells, caret `(11, 2)`, 8-cell rule | 14 columns, 10 fixed cells, caret `(12, 2)`, 9-cell rule |
| `render/buffer/tests/viewport.rs:10-34` | 11-column terminal, text from cell 5, caret `(9, 0)` | 12 columns, text from 6, caret `(10, 0)` |
| `render/buffer/tests/viewport.rs:49-55` | selection at `bg(5..)` | `bg(6..)` |
| `render/terminal_tests.rs:125` and its cell reads | `Screen::new(9, 4)` | `Screen::new(10, 4)`, cells + 1 |
| `render/frame_tests.rs:431-433` | `grid[(5, row)]` | `grid[(6, row)]` |
| `render/buffer/directory/tests.rs:84-115, 142` | rows `"   2 README.md …"` | one more blank after the number; check the size/age columns at the same pane width |
| `render/buffer/directory/tests.rs:93-95, 159, 166` | `grid[(5, y)]` | `grid[(6, y)]` |
| `headless.rs:144-153` | `"  1   1 fn a() {}"`, `"▎      2 fn new() {}"`, `"▎  2     fn old() {}"`, `"   ⋮   ⋮ @@"` | one more blank before the text in each |
| `render/picker/preview.rs:276-286, 335, 376, 385` | `"▌ 100 │ needle"`, `"  99 │ plain"`, `"   1 │ plain"`, `"▌ 42 │"`, `"│ fn dispatch"` | `"▌100  needle"`, `" 99  plain"`, `"  1  plain"`, `"▌ 42  fn dispatch"` |
| `render/picker/preview.rs:404-412` | tier widths | new full width |

Already written against the engine and expected to pass unchanged:
`block_frame_tests.rs` (`check`, `caret_truth`), `viewport.rs:388-414`,
`tests/documentation.rs:39`, `tests/collection.rs:31-42`,
`tests/syntax_stability.rs` (`content_start`),
`tests/terminal_editor/harness.rs:341` (`numbered_line`). The list above is
from a search for literals; the suite is the authority — any other failure is
a literal this table missed, fixed the same way.

**Goldens.** Every golden that shows a numbered row changes; regenerate with
`STROP_UPDATE_GOLDEN=1 cargo test -p strop` and read the diff: each numbered
row gains one blank cell after the number (or loses ` │` in a preview), its
style runs shift by one, and nothing else. Families: `editor`, `split*`,
`terminal*`, `collected`, `collection*`, `git-diff*`, `git-hunk`, `folder*`,
`help`, `review*`, `recovery`, `substitute`, `ex-visual`, `echo*`,
`completion`, `actions`, `locations`, `grep*`, `files`, `modeline-*`,
`which-key`, and any other picker with a preview. `git-status`, `git-log*`,
`git-files`, `git-branches` and `welcome` must **not** change (list gutter or
no gutter); a diff there is a bug.

### 8.5 Tests to add

1. **Geometry** (`prepare/geometry.rs`): `number_gutter` is 6 for 1, 99 and
   999 lines, 7 for 1000, 8 for 10000; a diff with three-digit numbers is 10.
2. **The text column** (`render/buffer/tests/viewport.rs`): a 20-line file at
   30×6 paints `" "," "," ","1"," "," ","f"` on row 0 and the terminal cursor
   is on `(6, 0)`; after `l` on `(7, 0)`; after `G` on `(6, last)`.
3. **The gap is blank and painted** (`block_frame_tests.rs`, in `check`): for
   the lines, source and pair gutters, on every row that shows a number the
   `NUMBER_GAP` cells before `left_inset` are blank and the cell before them
   is a digit; on the cursor row their background is `bg.cursorline`.
4. **Sideways scroll**: with `hscroll > 0` the gutter cells are the same as
   with `hscroll == 0` and the caret is `left_inset + column − hscroll` (the
   two edited viewport tests cover it; keep both).
5. **Signs do not move text**: the same file with and without a diagnostic on
   line 1 has the same text column (`diagnostics.rs`).
6. **Preview**: no `│` in any row's first `gutter` cells; a preview row's text
   starts at `number_gutter(lines)` when the pane is wide enough; the focused
   row has `▌` in cell 0.
7. **Blame on**: the caret is `22 + number gutter + column`
   (`frame_tests.rs`, next to `gutters_and_sidebar_render`).

Not added, because the behaviour does not exist: a mouse hit-test (no mouse
input; when it lands it must subtract `left_inset`, nothing else) and wrapped
rows (§5.3).

### 8.6 Words

- `plans/0100-surfaces.md` §3.5: one amendment line under the table —
  "*Amended by [0105](0105-the-number-gap.md): a number gutter ends in two
  blank cells.*"
- `plans/STATUS.md`: move 0105 to in force with the release.
- `CHANGELOG.md`: one line — "Two cells between line numbers and text; the
  picker preview uses the buffer's gutter."
- `docs/`: nothing describes the gap; no change. The site's frames are
  recaptured from the release binary as usual.

### 8.7 Order and gate

One change: engine constant, gutter paint, preview, tests, goldens. No kernel
code moves, so `proofs/` is untouched. Done when
`docker compose run --build --rm test` is green.

## 9. Taste questions for the owner

1. **Two cells or three?** Plan: two. Three is `NUMBER_GAP = 3` and a golden
   regeneration; look at a real file in Kiln before asking for it.
2. **Does the diff take the gap too?** Plan: yes — one rule for every number
   that sits left of text. The alternative keeps diffs at one cell and saves a
   cell in the narrow commit view.
3. **Does the preview lose its `│`?** Plan: yes; it is the last gutter bar in
   the product and it sits one cell from the picker's own divider. If you want
   the rootle shape kept, drop §8.3 and its test rows; nothing else depends on
   it.
4. **The gap stays two cells in a narrow pane.** Plan: yes. The alternative
   (one cell under some width) moves the text when a split is resized.
5. **The indent guide at text column 0.** With two cells it no longer reads as
   a separator, so the plan leaves it. If it still bothers you after this
   lands, the follow-up is to skip the first guide level — a separate change.
6. **Number ink.** Kiln's numbers and comments are nearly one grey. The plan
   lets distance separate them and changes no ink. Say so if you would rather
   have Kiln's `fg.muted` or its comment colour moved apart; that is a palette
   amendment to 0088, not part of this plan.
