# 0098 — The modeline: what it shows, where, and what gives way

- Status: **in force**; amends [0088](0088-the-look.md) §2 (the chip is 10 cells,
  not 8) and [0032](0032-modeline-and-git-polish.md) (truncation order).
- Source: the owner's note — "UI spacing for command card in the modline isn't
  well positioned, we need to revisit spacing in the modeling and polish it" —
  widened by the owner to the modeline's design as a whole: what it shows and in
  what priority, grouping, the cards beside it, emphasis, every width, and how
  it changes between states.

The look stays 0088's: one quiet row on the surface plane, one accent, four
greys, keycaps the only chip. Nothing here adds a colour, a separator glyph or
a new widget. This plan is about *position*: where each thing sits, how far it
is from its neighbour, and what moves when something changes.

## 1. What was rendered

Every state below was painted through `TestBackend` at 44, 80, 120 and 200
columns, with a short path (`src/main.rs`) and a long one
(`crates/strop/src/render/statusline/some_long_module_name.rs`), a 24-cell
branch name and three diagnostics: Normal, Insert, Visual, a pending operator,
a count, a register, `g`, the leader, marks, macro recording, `:`, `:w`,
`:%s/…`, `/`, `f`, `ct`. The grids are 0.62.0's.

### 1.1 The row

```
 NORMAL   feature/modeline-spacing  src/main.rs                  ●1 ▲2  6:1  15%
PENDING   feature/modeline-spacing  src/main.rs                d  ●1 ▲2  1:1  2%
PENDING   feature/modeline-spacing  src/main.rs               d2  ●1 ▲2  1:1  2%
PENDING   feature/modeline-spacing  src/main.rs           spaceg  ●1 ▲2  1:1  2%
COMMAND   feature/modeline-spacing  src/main.rs                   ●1 ▲2  1:1  2%
```

- **M1 — the chip's label touches the chip's edge.** The chip is 8 cells and
  the label is centred, so `NORMAL` has one cell either side but `PENDING`,
  `COMMAND`, `V-BLOCK`, `BUFFERS` and `SYMBOLS` sit hard against the left edge
  of the coloured block (and of the screen), and `TERMINAL` and `BRANCHES` fill
  it with no padding at all. Pressing `d` or `:` — the two most common
  transitions — is exactly when the label loses its padding.
- **M2 — the pending keys grow backwards.** The keys you are typing are
  right-aligned against the diagnostics, so `d` → `d2` → `d2i` pushes the `d`
  left one cell per key. Text you type should grow to the right from a fixed
  column, as Vim's `showcmd` field does. It also reads as one more readout in
  the `●1 ▲2  1:1  2%` cluster instead of as the thing in progress.
- **M3 — `spaceg`.** The leader prefix is printed with its tokens run together.
- **M4 — the right side shifts while you move.** `6:1  15%` is as wide as its
  digits. Moving from line 9 to 10, from column 9 to 10, or from 9 % to 10 %
  moves the diagnostics, the pending keys and everything else on the right by a
  cell. The `%` also touches the last screen column.
- **M5 — whole groups vanish while cells sit empty.**

  ```
   NORMAL   feature/modeline-spacing  …ne/some_long_module_name.rs ●1 ▲2  6:1  15%
  PENDING   some_long_module_name.rs                         space  ●1 ▲2  1:1  2%
  ```

  Pressing `Space` needed five more cells. The directory was dropped, that was
  not enough, so the whole 24-cell branch was dropped too — and the filename
  jumped 26 cells left with 25 blank cells beside it. Segments are dropped
  whole, in a fixed chain, and nothing is given back. The chip was made
  fixed-width "so the path never jumps"; the truncation chain undoes that.
- **M6 — a long footer evicts everything and repeats the card.**

  ```
  COMMAND   main.rs 40 substitutions · 29 offscreen · 40 changed lines · lin…  1:1
  ```

  While `:%s/…` is typed the substitution summary takes the modeline, pushing
  out the branch, the directory, the diagnostics and the percentage. One cell
  separates it from the filename (the only place the row uses a 1-cell gap
  between groups). The same sentence is already in the card.
- **M7 — recording a macro shows nothing.** `qa` echoes `recording @a` once;
  the next key clears the echo and nothing on screen says the keys are being
  recorded. Vim keeps `recording @a` up until `q`.
- **M8 — terminal hints are spaced differently from every other group**: one
  cell before the keycap, three after it.

### 1.2 The cards beside the row

```
   9 let value_8│  s  hunk: stage                           │
  10 let value_9│ +5 more                                   │
  11 let value_1╰───────────────────────────────────────────╯
PENDING   feature/modeline-spacing  src/main.rs           spaceg  ●1 ▲2  1:1  2%
```

- **C1 — the key card sits on the modeline.** Its bottom border is on the row
  directly above. Both are on the surface plane, so they read as one block. The
  code reserves "one row of breathing room"; the arithmetic puts that row above
  the card, not below it.
- **C2 — `+5 more` starts one cell left of the keys** it continues.

```
╭ command ─────────────────────────────────────╮
│ : w▏                                         │
│  write  :w[!] [file]: write (! over a file…  │
│  wall   write every modified buffer          │
╰──────────────────────────────────────────────╯
╭ command ─────────────────────────────────────╮
│ : %s/alpha/omega/g▏                          │
│ 40 substitutions · 29 offscreen · 40 changed │
│ -let value_0 = alpha + beta * 0;             │
```

- **C3 — three left edges in one card.** The sigil is at column 0 of the card's
  content, completion names at column 1, what you typed at column 2, the
  substitution summary and the changed lines back at column 0.
- **C4 — two right edges.** Completion docs and the search count stop one cell
  short of where the summary and the changed lines stop.
- **C5 — the summary is cut without `…`** (`40 changed │`), the one place in
  the card that clips silently.

## 2. How others do it

- **Vim** keeps `showcmd` in a fixed 10-cell field, left-aligned, at a fixed
  column; the ruler beside it is a fixed-width field too. Nothing moves while
  you type a command. `recording @a` stays in the mode area until `q`.
- **Helix** (`helix-term/src/ui/statusline.rs`) builds the row from three
  lists of named elements (left, centre, right); every element pads itself with
  one cell either side, and the mode is always three letters, so its width is
  constant. Its key popup (`ui/info.rs`) anchors bottom-right, above the
  statusline.
- **lualine** pads every component one cell either side, formats the location
  as `%3l:%-2v` and the progress as a fixed 3–4 cells so neither shifts its
  neighbours, and puts progress *before* location. Components hide by a width
  condition, least important first.
- **Zed** truncates the branch name with an ellipsis long before it hides it,
  and keeps the cursor position at the far right.
- **which-key.nvim** floats its window with a one-row margin above the
  statusline in its default preset.

What we take: fixed-width fields for anything that changes while you type or
move (Vim, lualine); shrink a long name before hiding a segment (Zed); one
padding rule for every element (helix, lualine); a margin between a floating
card and the row (which-key).

## 3. The rules

### 3.1 One spacing scale

| Cells | Where |
|---|---|
| 0 | a mark glued to its owner: `main*`, `dir/` + `name` |
| 1 | inside a group: `name ●`, `●1 ▲2`, `REC @a`, the pad inside the chip and at the row's right edge |
| 2 | between groups, and the least gap between the two halves |

There is no other distance in the row. The flexible gap between the halves is
whatever is left.

### 3.2 The row, left to right

```
  NORMAL    REC @a  main*  src/render/statusline.rs ●      d2        LSP!  ●1 ▲2  1/4   50%  10:1
└─ chip ─┘ └ rec ┘ └ git ┘ └───────── file ────────┘      └ keys ──┘ └── signals ──┘ └── where ───┘
└──────────────── what this is ───────────────────┘      └──────── what is happening ─────────────┘
```

- **Left half: identity.** Mode chip, recording, branch, file. These change
  when you change mode, branch or buffer.
- **Right half: state.** Pending keys, terminal hints, service badge,
  diagnostics, indent (only when it differs from the default), search count,
  percentage, `line:col`.

**The chip is 10 cells**: the longest label (`TERMINAL`, `BRANCHES`) plus one
cell either side, centred, the odd cell on the right. Every label has padding on
both sides (M1).

**Recording** is `REC` in `diag.error` bold, as the terminal capture badge
already is; a macro adds its register: `REC @a` (M7).

**The pending keys have a slot**: 8 cells at the start of the right half,
left-aligned, in the accent. `d` → `d2` → `d2i` grows to the right and the `d`
does not move (M2). The slot exists only while something pends; a footer longer
than the slot takes the cells it needs. Prefix tokens are separated: `space g`
(M3). While the `:` card shows a substitution's summary, the modeline does not
repeat it (M6) — one fact, one place.

**Where you are is a fixed field** (M4): the percentage is 4 cells,
right-aligned (`  2%`, ` 50%`, `100%`); the line is right-aligned to the width
of the buffer's last line number; the column is left-aligned in at least two
cells, then one cell of edge padding. The colon stays in one column while you
move. Percentage comes before `line:col` (lualine's order) so the column's slack
falls at the screen edge, where it is invisible, instead of opening a hole
between two readouts. The search count pads its current index to the total's
width (` 3/12`).

**Emphasis** stays inside 0088's four greys: the filename is `fg.text`, the
directory `fg.secondary`, branch, percentage, search count and indent
`fg.muted`. `line:col` moves from `fg.muted` to `fg.secondary`: it is the
readout on the right you actually look for, and it now reads one step above the
percentage beside it. The pending keys are the accent (rule 2, unchanged).

### 3.3 What gives way: shrink before drop

Each part of the row has a *floor* (the fewest cells worth showing) and a
*natural* width. Layout is two passes over two tables — data, not a chain of
`if`s:

1. **Keep.** In keep order, a part is shown if its floor and its separator
   fit in what is left.
2. **Grow.** What is left after every floor is handed out in grow order, each
   part taking up to its natural width.

| Keep order (first = last to leave) | Floor |
|---|---|
| chip | 10 |
| `line:col` | whole |
| service badge (`LSP!`) | whole |
| filename with `●`, `[RO]`, `N×` | 4 cells of name + marks |
| pending keys | 3 |
| `ssh:host` | 4 |
| `REC` | whole |
| terminal attention, hints | whole badges; the first hint |
| diagnostics | whole |
| search count | whole |
| indent | whole |
| branch | 8 cells of name + marks |
| percentage | whole |
| directory | 4 |

Grow order: filename (to two thirds of the spare cells, so a huge name cannot
starve the footer), pending footer, filename again, `ssh:host`, hints (whole
chords, rule 10), branch, directory, then the pending slot's padding.

So the directory still yields first and the filename last, but a long branch
now shortens to `feature/…` before it disappears, a part that was dropped
comes back as soon as its floor fits, and typing a key takes only the cells the
key needs (M5). When not even chip, position and badge fit, the row is the bare
`chip · line:col` of 0032, with the safety marks inside the chip.

### 3.4 The cards

- **The key card keeps one empty row above the echo row / modeline** (C1). It
  stays centred (0068 C1). `+N more` starts in the key column (C2).
- **The `:` / `/` card stays top-centre, one row below the top edge**, at
  0088's 60 % width. It is a typed field: it must not move or resize under the
  caret, and the top is the one place neither the echo row nor the key card ever
  occupies. The two cards now have the same margin from the edge they hang
  from.
- **Inside the `:` card there is one text column** (C3): the sigil and the
  row markers (`▌`, `-`, `+`) live in a two-cell gutter; what you type,
  completion names, the summary and the changed lines all start in the column
  after it. Every row may use the full width to one cell before the border
  (C4), and everything that is cut says so with `…` (C5).

## 4. Rejected

- **Bar or powerline separators** between groups. 0088 has no separator glyph;
  two cells and a change of ink do the job.
- **Left-aligning the chip label** (`NORMAL` then three cells of colour). The
  first letter would never move, but the commonest chip would look lopsided.
  Centring moves the label by one cell at most, on a keypress that changes its
  word and colour anyway.
- **A permanently reserved pending slot**, as Vim has. It would cost eight
  cells of directory on every narrow row to keep them blank almost always. With
  shrink-before-drop, a key typed on a full row costs its own width and no more.
- **Moving the key card to the bottom-right, over the pending keys** (helix).
  The owner asked for it centred in 0068.
- **Moving the `:` line into the modeline row** (Vim's place for it). 0091 B2/B6
  settled on the card; completions and the substitution preview need the rows.
- **Hiding the pending keys while the key card is up.** The card is not drawn
  on very small terminals; the slot is the one place that is always there.
- **A centre section** (helix has one). Two halves and one flexible gap are
  enough for one file per row.

## 5. Test contract

- Golden frames (`render/golden/`) for the row in six states — Normal with
  branch, diagnostics and a long path; a pending operator with a count; the
  leader with its card; macro recording; `:w` with completions; a search count —
  at 48, 80, 120 and 200 columns, plus the existing 140×40 and 80×24 scenes.
- The allocator, as a pure function: never hands out more cells than the row
  has; never gives a part less than its floor or more than its natural width;
  a part is absent only if its floor did not fit; widening the row never
  shrinks a part that was shown.
- The chip is 10 cells with padding either side in every mode.
- Typing `d`, `2`, `i` leaves the first key's column unchanged; on a row with
  spare cells nothing else on the row moves.
- Moving the cursor across a digit boundary (line 9 → 10, column 9 → 10) moves
  no other cell of the row.
- A long branch is shortened before it is dropped; the filename survives; the
  row fits at every width from 1 to 200.
- The modeline does not repeat the substitution summary; `REC @a` shows from
  `qa` until `q`.
- The key card leaves the row above the modeline empty; the `:` card's typed
  text, completion names and summary share one column.
