# 0100 — Surfaces: one look for the collection and the git buffers

- Status: **in force** (steps 1–9 of §6.4, log step A). Log step B and the
  fuller commit view of §4.4 wait for the typed data of §7 (0101). See §11
  for the implementation record.
- Date: 2026-11-12
- Source: the maintainer's note — "what about UI polish for it? I feel its UI
  isn't nice tbh, with no proper definition for the blocks ... git and commit
  viewer as well needs UI polish". "It" is the collection (0097). The git part
  covers status, log, file history, commit files, the diff and hunk views and
  the commit message buffer (0096, 0010, 0032).
- Builds on [0088](0088-the-look.md) (palette, four greys, one accent, the
  ruled heading) and [0098](0098-modeline-design.md) (the 0/1/2 spacing scale,
  layout as one table). Adds no colour and no new plane.

## 1. What was rendered

Every surface was painted through `TestBackend` at 80×24, 120×40 and 200×50 in
`hone` and `vellum`, from real repositories and a real search:

- a collection of 27 hits in 4 files: a 97-cell path, a hit on line 1 and on
  the last line of a file, three hits on adjacent lines, two hits on one line,
  a 180-cell line;
- a repository with a merge, a renamed file, a changed binary file and a
  commit subject of 110 cells; a working tree with staged, unstaged, deleted,
  untracked and binary entries and a three-hunk diff expanded in the status
  buffer.

The grids below are 0.63.0's, at 80 columns. Both themes show the same shapes;
nothing here is a one-theme problem.

### 1.1 The collection

```
     search token — 27 hits in 4 files                                         ▮
     ╭─ crates/strop-engine/src/editor/collections/tests/a_very_long_module_nam▮
   1     let value_1 = compute(1);                                             ▮
   2     let value_2 = compute(2);                                             ▮
   3     let token_3 = parse_token(input, 3);                                  ▮
   4     let value_4 = compute(4);                                             ▮
   5     let value_5 = compute(5);                                             ▮
     ⋮ 12 lines not shown                                                      ▮
  18     let value_18 = compute(18);
```
```
  40     let token_40 = parse_token(input, 40);
     ╰────────────────────────────────────────────────────────────────────────╯
     ╭─ src/b.rs ── 2 hits ───────────────────────────────────────────────────╮
   4     let value_4 = compute(4);
```

- **S1 — a box with no sides.** The card has four corners and a top and bottom
  rule, and nothing joins them: the body rows have no left or right edge. The
  corners promise a box the body does not draw. This is the "no proper
  definition".
- **S2 — two blocks touch.** `╰───╯` is followed on the next row by `╭───╮`.
  There is no space between files; the eye sees one double rule, not an end
  and a start.
- **S3 — the rules do not start where the block starts.** Line numbers sit in
  columns 1–3, the rules start at column 5. The number column, which is part
  of the block, is outside its outline.
- **S4 — a long path is cut with no `…`**, and takes the hit count and the
  corner with it. The longest heading is the one that says least.
- **S5 — the heading here is not the heading in the list it came from.** Search
  draws ` docs/remote.md ──────── 2` (0088's ruled heading). `Ctrl-O` turns the
  same file into `╭─ docs/remote.md ── 2 hits ───╮`. Same thing, two drawings,
  two code paths.
- **S6 — the gap is weaker than the code around it.** `⋮ 12 lines not shown`
  is muted text in the code column. Nothing marks the jump in the number
  column, where the jump actually is.
- **S7 — no cursor line.** File buffers paint the cursor line; the collection
  does not. Only the line number changes ink.
- **S8 — the title is glued to the first card**, in plain text ink, with no
  space under it.
- **S9 — `· modified` and `· read-only`** are words where every other surface
  uses the modeline's marks (`●`, `[RO]`).
- **S10 — drawing glyphs are buffer text.** `╭─ `, ` ── `, `⋮ ` and `╰` are
  in the buffer; the rest of the rule is painted. Yanking a heading copies
  `╭─ src/a.rs ── 3 hits`.

### 1.2 Status

```
   1 Head: main                                                                ▮
   2                                                                           ▮
   3 Staged (3)                                                                ▮
   4 A docs/guide.md                                                           ▮
   5 D docs/new-name.md                                                        ▮
   6 A docs/renamed-again.md                                                   ▮
   7                                                                           ▮
   8 Unstaged (3)                                                              ▮
   9 M logo.bin                                                                ▮
  10   binary file                                                             ▮
  11 D notes.md                                                                ▮
  12 M src/lib.rs                                                              ▮
  13 @@ -1,4 +1,4 @@
  14 -    let value_1 = compute(1);
  15 +    let first = compute(1);
  16      let value_2 = compute(2);
```

- **G1 — four levels in one column.** Section, file, hunk header and diff line
  all start at column 5. A context line (`     let value_2…`) and a file row
  (`M src/lib.rs`) differ by one space.
- **G2 — an expanded file has no end.** Nothing shows which rows belong to
  `src/lib.rs`; the diff simply stops at the next blank row or the next file.
  Nothing shows which files are expanded.
- **G3 — the gutter counts buffer lines.** `1 … 16` number rows of a list.
  They mean nothing, cost five cells, and sit where the diff view shows real
  old/new line numbers.
- **G4 — the same diff, drawn differently.** Here a changed line is all green
  or all red text with a `+`/`-`. In the diff view it is syntax-coloured text
  on a tint with a `▎` sign and intra-line emphasis.
- **G5 — the band stops short.** A hunk header's band starts after the gutter,
  except on the cursor row, where it covers the gutter too.
- **G6 — a section heading is bold text and nothing else**; `M` (modified) is
  the same grey as `?`.

### 1.3 Log and file history

```
   1 * 0d21f9e Tarek Example · 1 second ago · notes: revision 5
   7 * 19b4e9a Tarek Example · 1 second ago · rename the doc, change the logo,
   8 *   0d964a0 Tarek Example · 1 second ago · Merge branch 'feature/surfaces'
   9 |\
  10 | * e8c1034 Tarek Example · 1 second ago · layout: one token
  11 | * ff9ea27 Tarek Example · 1 second ago · surfaces: parse tokens in three
```

- **L1 — the subject comes last and is cut first**, with no `…`. Author and age
  take 30 cells in front of the one field you read.
- **L2 — nothing lines up.** A merge row's sha starts two cells right of its
  neighbours; a longer author name moves the subject.
- **L3 — the accent is on every row.** Every sha is accent bold, and the first
  graph lane is the accent too. 0088 rule 2 gives the accent one job (where am
  I). On this screen it marks nothing.
- **L4 — no refs.** `HEAD`, branches and tags are not shown. You cannot see
  where `main` is.
- **L5 — line numbers again** (G3).
- **L6 — file history does not name its file.** The buffer and the modeline
  say `git log ·file`.
- **L7 — the age is git's sentence** ("35 seconds ago"), so the row changes
  width over time and cannot be pinned in a golden.

### 1.4 Commit files

```
   1 commit 19b4e9aec5
   2
   3 docs/new-name.md                                                     +0 -0
   4 logo.bin                                                             +0 -0
   5 src/lib.rs                                                           +2 -0
```

- **F1 — the commit viewer does not show the commit.** No subject, author or
  date. The one thing shown is ten characters of sha.
- **F2 — a rename and a binary change both read `+0 -0`.** The rename shows
  only its new name. Status letters (`A`/`M`/`D`) appear in the status buffer
  and not here.
- **F3 — the counts are at the far edge.** At 200 columns the `+2 -0` is 180
  cells from the name it belongs to.
- **F4 — no heading, no total**, and line numbers again.

### 1.5 Diff and hunk

```
 docs/          │         src/lib.rs +2 -0
   new-name.md  │         @@ -58,3 +58,5 @@
 logo.bin       │  58  58     let value_58 = compute(58);
 src/           │  59  59     let value_59 = compute(59);
▌  lib.rs       │  60  60     let value_60 = compute(60);
                │▎     61     let token_61 = parse_token(input, 61);
```
```
         hunk +1 -1
         @@ -27,7 +27,7 @@
  27  27     let value_27 = compute(27);
```
```
 docs/          │         logo.bin +0 -0
   new-name.md  │~
▌logo.bin       │~
```

- **D1 — the file heading and the hunk heading are the same thing**: text on
  the same band, at the same column. File and hunk do not read as two levels.
- **D2 — the hunk view is titled `hunk`**, not with its file. It is also the
  only diff with no syntax colour.
- **D3 — a binary file is an empty buffer** titled `+0 -0`.
- **D4 — bands** as G5.

The diff body itself — two number columns, `▎`, quiet tints, intra-line
emphasis — is good and stays.

### 1.6 Commit message

```
   1 surfaces: one visual language
   2
   3 # Write the message above; lines starting with # are left out.
   4 # :wq commits, :q! aborts.
```

- **M1 — comment lines are comment-coloured but upright**; every other comment
  in the editor is italic (0088 rule 7).
- **M2 — nothing marks a subject that is too long** or a second line that is
  not empty, the two mistakes this buffer exists to avoid.

It is a file and reads like one. It needs the least.

### 1.7 Between the surfaces

| The same thing | Collection | Status | Files | Diff |
|---|---|---|---|---|
| a file's heading | `╭─ path ── N hits ─╮` | `M path` | `path` … `+N -M` | `path +N -M` on a band |
| a jump in line numbers | `⋮ N lines not shown`, muted | `@@ … @@` on a band | — | `@@ … @@` on a band |
| a count | in the heading, as words | `(3)` after the name | far right | after the name |
| the gutter | source line numbers | buffer line numbers | buffer line numbers | old and new numbers |
| the cursor row | accent number only | band + accent number | band + accent number | accent numbers only |
| a changed line | — | coloured text, `+`/`-` | — | tint, `▎`, emphasis, syntax |
| modified / read-only | words | — | — | — |

Seven things, drawn up to four ways. Each surface was written by hand: the
card in `buffer/rows.rs`, status in `diff/status.rs`, log and files in
`diff/list.rs`, the diff bands in `diff.rs`, the search heading in
`picker/rows/search.rs`. `render_pane` picks between them with an `if`/`match`
chain whose order matters (AGENTS.md: "an if-chain whose branch order is
load-bearing is over the limit").

## 2. How others do it

- **Zed's multibuffer.** Each file has a header row with the path; excerpts of
  one file are separated by a thin divider with the skipped range; the gutter
  shows the file's own line numbers. No box. *Take:* a header row per file,
  the file's line numbers, a quiet divider at each jump. *Leave:* the sticky
  header and the expand buttons — they are widgets, not buffer rows.
- **Magit / Neogit.** Section headings with a count; file rows under them;
  hunks inline under the file; the current hunk is highlighted as a block.
  *Take:* counted section headings, nesting by position. *Leave:* highlighting
  a whole section (0088 has one cursor-row idiom) and transient menus (0096).
- **lazygit.** The log is columns: sha, author, graph, subject, each at a fixed
  position; long subjects end in `…`. *Take:* fixed columns and the ellipsis.
  *Leave:* a colour per author and bordered panels.
- **tig.** `date author graph [refs] subject`, fixed widths, refs in brackets
  in the row. *Take:* refs in the row, in words, not as chips.
- **delta.** A file is a ruled heading; a hunk is a smaller heading; two number
  columns; intra-line emphasis. *Take:* a rule for the file and a quieter mark
  for the hunk — two levels drawn two ways. strop's diff body already matches.
- **Helix / Telescope pickers, and strop's own Search.** Group heading = name,
  rule, count; rows under it carry line numbers. *Take:* this is already
  0088's ruled heading. The collection should keep it instead of swapping it
  for a card.

What they share: a block starts with a heading row that spans the width, ends
at empty space or the next heading, and nobody draws a box around text you
edit.

## 3. The language

One sentence: **a block is a ruled heading, its rows, and one empty row; a jump
inside a block is a band; the buffer holds words, the renderer draws lines.**

### 3.1 Row kinds

Every row of every surface in this plan is one of six kinds. A surface says
which kind each of its rows is (typed, from its row model); one renderer draws
each kind.

| Kind | What it is | How it is drawn |
|---|---|---|
| **title** | says what the buffer is: `search token — 27 hits in 4 files`, `Head: main` | text column; the subject bold `fg.text`, labels `fg.muted`, the rest `fg.secondary` |
| **heading** | starts a block: a file, a status section, a commit | a rule across the whole pane with the name set into it (§3.2) |
| **break** | a jump inside a block: a gap between excerpts, a hunk header | `bg.surface` band across the whole row, gutter included; `⋮` in each number column; the label `fg.muted` |
| **row** | one entry of a list: a status file, a changed file, a commit | sign column + text; a right-hand fact where the surface has one |
| **line** | one line of a file or of a diff | number gutter + text, as a file buffer draws it |
| **blank** | space | nothing |

### 3.2 The heading

```
──── src/b.rs ● ──────────────────────────────────────────────────────── 2 hits
└gutter┘└name┘└marks┘└──────────────── leader ────────────────────────┘ └fact─┘
```

- The rule starts at the pane's first column and runs through the gutter, so
  the number column is inside the block (S3). The last gutter cell is a space.
- The **name** starts at the text column — the same column as the buffer text
  under the cursor, so the caret is where the letters are.
- **Marks** are the modeline's: `●` in `diag.warning` for modified, `[RO]` in
  `fg.muted` (S9). One cell before each.
- The **leader** is `─` in `fg.faint`, with one cell either side.
- The **fact** is right-aligned to the last content column (the scrollbar
  column stays reserved): a count, `+N -M`, an author and age.
- Inks: a file name follows `render/file_name.rs` in its heading role
  (directory `fg.secondary`, name bold `fg.text`); any other name is bold
  `fg.text`; the rule `fg.faint`; the fact `fg.secondary`, with `+N` and `-M`
  in the diff inks. In the block the cursor is in, the rule is `fg.muted`
  (0088 rule 3: focus is the border going to muted); in every other block the
  name's bold ink drops to `fg.secondary`.

This is the heading Search already draws, with the rule extended left through
the gutter. Search's own heading has no gutter, so it does not change.

### 3.3 How a block starts, ends and separates

- It **starts** at its heading.
- It **ends** at the last row before the next blank row. There is no bottom
  rule: the next heading's rule, or the end of the buffer, closes it. Corners
  are gone — a corner needs a side, and a side cannot be drawn beside text
  that scrolls sideways and is edited (S1).
- Blocks are **separated** by exactly one blank row. The title is followed by
  one blank row (S2, S8).

A collection costs the same rows as today: the old bottom border becomes the
blank row.

### 3.4 Hierarchy

Three levels, three different means, so they cannot be confused:

1. **block** — a rule (heading);
2. **jump** — a band (break);
3. **line** — text.

A fourth, only in the status buffer: rows nested under a list row (the diff of
an expanded file) carry a **rail**, `│` in `fg.faint`, in the sign column from
the first nested row to the last. It shows which file the rows belong to and
which files are open (G1, G2). The file row itself has no rail.

### 3.5 Gutters

The gutter is chosen per surface, as data, in one place
(`prepare/geometry.rs`), because the caret needs the same answer.

| Gutter | Cells | Surfaces | Cursor row |
|---|---|---|---|
| lines | sign + digits + 1 | file buffers (unchanged) | accent number, `bg.cursorline` |
| source | sign + digits + 1 | collection: the *file's* line number; blank on title, rule on headings, `⋮` on breaks | accent number, `bg.cursorline` (S7) |
| pair | sign + old + 1 + new + 1 | diff, hunk (unchanged) | accent numbers, `bg.cursorline` over context rows; tinted rows keep their tint |
| list | 2: sign + 1 | status, log, file history, commit files | `▌` in the accent in the sign column + `bg.cursorline` across the row |

The list gutter has **no line numbers** (G3, L5, F4): the rows are entries, not
lines. Its cursor row is the directory buffer's idiom, which these buffers
most resemble. Counts still work (`5j`, `12G`); the modeline shows the line.

On a break row, `⋮` sits in the last digit column of each number column, in
`fg.faint`: the mark is where the numbers jump (S6).

### 3.6 Spacing (0098's scale)

| | Across | Down |
|---|---|---|
| 0 | a mark glued to its owner: `dir/`+`name`, `▌`+row | heading → its first row; row → break → row |
| 1 | inside a group: `M path`, `name ●`, either side of a leader, `+2 -0` | between blocks; under the title |
| 2 | between groups: a row's text and its fact, at least; `3 files  +2 -0` | — |

No other distance appears.

### 3.7 Colour roles

Only tokens that exist. Nothing new.

| Role | Token |
|---|---|
| heading rule; rail; `⋮` | `fg.faint` (`fg.muted` on the rule of the block the cursor is in) |
| heading name, title subject | `fg.text` bold (file names via `file_name.rs`) |
| facts: counts, author, age, totals | `fg.secondary` |
| break label; `Head:` / `Upstream:` labels; sha separators | `fg.muted` |
| break band | `bg.surface` |
| cursor row | `bg.cursorline`; `▌` and the current line number in the accent |
| the branch you are on (`Head: main`, `HEAD ->` in the log) | accent bold — "where am I" |
| search hits; the current hit | accent bold; + underline (unchanged) |
| added / deleted: sign, `+N`/`-M`, the `+`/`-` prefix, letters `A`/`D` | `diff.add` / `diff.delete`; tints `diff.add_bg` / `diff.delete_bg` |
| letter `M` | `diff.change` (today `fg.muted`, the same as `?` — G6) |
| letter `?`, `binary`, notes | `fg.muted`; notes italic (rule 7: unavailable rows) |
| conflicted, modified mark `●` | `diag.warning` |
| a commit's sha | `fg.secondary` (today accent bold on every row — L3) |
| graph lanes | as today |

The accent leaves the sha and stays on: the cursor row's mark, the branch you
are on, search hits.

### 3.8 Words in the buffer, lines on the screen

**Decision: a row's buffer text is its words; everything drawn is render-only.**

- In the buffer: the path of a heading, a section's name, the title sentence,
  `12 lines not shown`, `@@ -1,4 +1,4 @@`, a status letter and path, a diff
  line with its `+`/`-`, a log row.
- Render-only: rules, leaders, `⋮`, rails, `▌`, padding, marks (`●`, `[RO]`),
  and a heading's or row's right-hand fact (`3 hits`, `+2 -0`, `3`).

So the collection's heading row holds `src/a.rs` (today `╭─ src/a.rs ── 3
hits`), its gap row holds `12 lines not shown` (today with a leading `⋮ `),
its bottom-border row (`╰`) becomes the empty row before the next heading, and
a status section holds `Staged` (today `Staged (3)`).

Why this way round:

- **A yank stays meaningful, and gets better.** `yy` on a heading copies the
  path. A visual yank across a collection copies paths, code and "12 lines not
  shown" — no box-drawing characters to delete after pasting. A yank in the
  status buffer is still a patch (0096).
- **Search and motions see words.** `/src\/a` finds the heading; `w` does not
  stop on `╭─`.
- **The caret is true.** The painted cells of a row begin with the buffer's
  own bytes at the text column; drawing is either in the gutter or after the
  text. This is the contract `diff/list.rs` already keeps for file rows ("the
  buffer row holds only the path; the stats are virtual"); this plan makes it
  the rule for every surface instead of one of three habits (S10).
- **Roles are typed already.** `CollectionRow`, `StatusRow` and `DiffRow` say
  what each line is; nothing reads the glyphs back. Edits to a heading, gap or
  blank row stay refused by role (0094), unchanged.

One known compromise, already present in commit-file rows: when a heading's
path must be clipped (§3.9), the painted name is shorter than the buffer text,
and the caret on that row is clamped to the painted cells.

### 3.9 Narrow widths

A heading is laid out by one pass over one table, as the modeline is (0098
§3.3). First row = last to leave; each part shrinks to its floor, then leaves,
before the part above it loses a cell.

| Part | Floor |
|---|---|
| gutter rule | whole |
| the name's last component (the file name) | 8 cells, cut at the end with `…` |
| marks | whole |
| fact | whole |
| the file name again: the rest it wants | |
| leader | 3 cells of rule + 1 either side; below that, a 2-cell gap |
| directory | 2 cells (`…/`), cut from the left (0088) |

```
──── crates/strop-engine/src/editor/collections/tests/a_very_long_module_name_for_projection_updates.rs ─────── 10 hits   120
──── …tions/tests/a_very_long_module_name_for_projection_updates.rs ─── 10 hits                                            80
──── a_very_long_module_name_for_proj…  10 hits                                                                            48
──── a_very_long_modu…  10 hits                                                                                            32
──── src/b.rs ● ──────────────────────── 2 hits                                                                            48
──── b.rs ● ──── 2 hits                                                                                                    24
```

Other rows:

- A **break** label and a **title** are cut at the last content column with `…`.
- A **list row** that does not fit ends in `…` in its last cell (L1). The text
  still scrolls sideways with the cursor, as any long line does.
- A **list block with facts** (commit files) takes its width from its content:
  the longest row + 2 + the widest fact, at least 48 cells, at most the pane
  (0088 rule 9). The heading's rule stops at the same column, so the counts
  sit beside the names at any terminal width (F3).
- A **log row** drops the author under 60 columns, then cuts the subject; the
  sha and age stay (§4.3).
- **Lines** are unchanged: they scroll.
- The diff's file sidebar is unchanged.

## 4. The surfaces, after

Grids are 80 columns wide; trailing blanks are not shown. The last content
column is 78; column 79 is the scrollbar's. Styles are §3.7.

### 4.1 Collection

Before: §1.1. After, the same scene:

```
     search token — 27 hits in 4 files

──── …tions/tests/a_very_long_module_name_for_projection_updates.rs ─── 10 hits
   1     let value_1 = compute(1);
   2     let value_2 = compute(2);
   3     let token_3 = parse_token(input, 3);
   4     let value_4 = compute(4);
   5     let value_5 = compute(5);
   ⋮ 12 lines not shown
  18     let value_18 = compute(18);
  19     let value_19 = compute(19);
  20     let token_20 = parse_token(input, 20);
  21     let token_21 = parse_token(input, 21);
  22     let token_22 = parse_token(input, 22);
  23     let value_23 = compute(23);
  24     let value_24 = compute(24);
   ⋮ 31 lines not shown
  56     let value_56 = compute(56);
  57     let value_57 = compute(57);
  58     let token_58 = parse_token(input, 58);
  59     let value_59 = compute(59);
  60     let value_60 = compute(60);
 › 27 hits in 4 files — ]q next, Enter opens the file, :w writes
  NORMAL    collection: search token                                 17%  12:9
```

Where two blocks meet, with a modified and a read-only file:

```
  38     let value_38 = compute(38);
  39     let value_39 = compute(39);
  40     let token_40 = parse_token(input, 40);

──── src/b.rs ● ──────────────────────────────────────────────────────── 2 hits
   4     let value_4 = compute(4);
   5     let value_5 = compute(5);
   6     let token_6 = parse_token(input, 6);
   7     let value_7 = compute(7);
   8     let value_8 = compute(8);

──── src/long.rs [RO] ─────────────────────────────────────────────────── 1 hit
   2     let value_2 = compute(2);
```

- Title: `search token` bold `fg.text`; ` — 27 hits in 4 files` `fg.secondary`.
- Row 12 (the cursor): `bg.cursorline` across the row, `20` in the accent, the
  hit `token` accent bold underlined; other hits accent bold.
- The two `⋮` rows: `bg.surface` from column 0 to 78; `⋮` at column 3 `fg.faint`;
  label `fg.muted`.
- The block holding the cursor has its rule in `fg.muted`.
- Buffer text of the rows shown: `search token — 27 hits in 4 files`, ``,
  `crates/…/a_very_long_module_name_for_projection_updates.rs` (whole), the
  code lines, `12 lines not shown`.

### 4.2 Status

Before: §1.2. After, cursor on `src/lib.rs`, which is expanded, as is
`logo.bin`:

```
  Head: main
  Upstream: origin/main, ahead 1

─ Staged ──────────────────────────────────────────────────────────────────── 3
  A docs/guide.md
  D docs/new-name.md
  A docs/renamed-again.md

─ Unstaged ────────────────────────────────────────────────────────────────── 3
  M logo.bin
│   binary file
  D notes.md
▌ M src/lib.rs
│ @@ -1,4 +1,4 @@
│ -    let value_1 = compute(1);
│ +    let first = compute(1);
│      let value_2 = compute(2);
│      let value_3 = compute(3);
│      let value_4 = compute(4);
│ @@ -27,7 +27,7 @@
│      let value_27 = compute(27);
│      let value_28 = compute(28);
│      let value_29 = compute(29);
  NORMAL    main  git status [RO]                                    30%  13:1
```

- `Head:` / `Upstream:` `fg.muted`; `main` accent bold; `origin/main`
  `fg.secondary`; `, ahead 1` `fg.muted` (as today).
- Section names bold `fg.text`; the count is the heading's fact.
- Letters: `A` `diff.add`, `D` `diff.delete`, `M` `diff.change`, `?` `fg.muted`.
  Paths in the list idiom (directory `fg.secondary`, name `fg.text`).
- Rail `│` `fg.faint` at column 0 on every nested row. On the cursor row the
  `▌` takes that cell.
- `@@` rows: `bg.surface` from column 0 to 78, ranges tinted as today.
- `-` / `+` rows: the diff view's drawing (§4.5) — tint across the row from
  column 2, the prefix in the diff ink, the text in `fg.text`, intra-line
  emphasis. No number columns: the list gutter has none.
- `binary file`, `no line changes`: `fg.muted` italic.
- Buffer text is today's, except `Staged (3)` → `Staged`.

At 48 columns only the rules shorten: `─ Unstaged ────────────────────────────────── 3`.

### 4.3 Log and file history

Before: §1.3. Two steps, because the second needs data this plan must not
fetch (§7).

**Step A — with today's rows** (`LogRow { text, sha }`):

```
▌ * 0d21f9e Tarek Example · 2 days ago · notes: revision 5
  * fd21dde Tarek Example · 2 days ago · notes: revision 4
  * 19b4e9a Tarek Example · 3 days ago · rename the doc, change the logo, add …
  *   0d964a0 Tarek Example · 3 days ago · Merge branch 'feature/surfaces'
  |\
  | * e8c1034 Tarek Example · 4 days ago · layout: one token
  | * ff9ea27 Tarek Example · 4 days ago · surfaces: parse tokens in three pla…
  * | c756836 Tarek Example · 3 days ago · notes: main moved on
  |/
  * 8cc6dd6 Tarek Example · 5 days ago · base: the first commit of the project
~
  NORMAL    main  git log [RO]                                       10%   1:1
```

List gutter, `▌` and band on the cursor row, `…` when cut, sha `fg.secondary`,
author, age and ` · ` `fg.muted`, subject `fg.text`. Three more cells of
subject, and the accent appears once. L1 is eased, L3 and L5 are fixed; L2, L4
and L7 are not.

**Step B — with typed rows** (graph, sha, refs, subject, author, time as
separate fields):

```
▌ *   0d21f9e  (HEAD -> main) notes: revision 5              Tarek Example   2d
  *   fd21dde  notes: revision 4                             Tarek Example   2d
  *   19b4e9a  rename the doc, change the logo, add a token  Tarek Example   3d
  *   0d964a0  Merge branch 'feature/surfaces'               Tarek Example   3d
  |\
  | * e8c1034  (feature/surfaces) layout: one token          Tarek Example   4d
  | * ff9ea27  surfaces: parse tokens in three places with…  Tarek Example   4d
  * | c756836  notes: main moved on                          Tarek Example   3d
  |/
  *   8cc6dd6  base: the first commit of the project         Tarek Example   5d
~
  NORMAL    main  git log [RO]                                       10%   1:1
```

At 48 columns the author leaves and the subject shortens:

```
▌ *   0d21f9e  (HEAD -> main) notes: revi…   2d
  | * ff9ea27  surfaces: parse tokens in …   4d
```

- Columns: list gutter; graph padded to the widest graph among the loaded
  rows; 1; sha (7); 2; refs and subject; then the fact — 2, author, 2, age
  right-aligned in 3 cells — ending at the last content column.
- Buffer text: `graph sha  (refs) subject`. Author and age are the row's fact,
  render-only (§3.8), computed from the commit time against the editor's
  clock, as the blame column does (`1s 5m 3h 2d 4w 6mo 1y`). That also makes
  the row stable enough to pin in a golden (L7).
- Refs `fg.secondary` bold in their parentheses; the branch `HEAD` points to
  in the accent (L4). No chips (rule 6).
- File history is the same renderer. Its buffer is named for the file:
  `git log · src/lib.rs` (L6).

### 4.4 Commit files

Before: §1.4. **With today's data** (`sha`, and `path`, `added`, `deleted` per
file):

```
─ commit 19b4e9aec5 ───────────── 3 files  +2 -0
▌ docs/new-name.md                         +0 -0
  logo.bin                                 +0 -0
  src/lib.rs                               +2 -0
~
~
  NORMAL    @19b4e9ae  git files [RO]                                 25%  2:1
```

- The buffer is one block; its first row is the heading. Name: `commit` in
  `fg.muted`, the sha bold `fg.text`. Fact: the file count `fg.secondary`, 2
  cells, the totals in the diff inks.
- Block width 48 here (§3.9): the longest row is short, so the floor applies.
- Buffer text: `commit 19b4e9aec5`, then one path per row. The empty second
  row of today's buffer goes (§3.6: nothing between a heading and its rows).

**With the commit's message and per-file status** (data request, §7):

```
─ commit 19b4e9aec5 ───────────── Tarek Example · 3 days ago
  rename the doc, change the logo, add a token

─ Files ───────────────────────────────────── 3 files  +2 -0
▌ R docs/old-name.md → docs/new-name.md                +0 -0
  M logo.bin                                          binary
  M src/lib.rs                                         +2 -0
```

The subject is bold `fg.text`; body lines, if any, follow it in `fg.text`
after one blank row. Letters as in status; `→` `fg.muted`; `binary` `fg.muted`.
This is the commit viewer F1 and F2 ask for.

### 4.5 Diff and hunk

Before: §1.5. After:

```
 docs/          │──────── src/lib.rs ──────────────────────────────────── +2 -0
   new-name.md  │   ⋮   ⋮ @@ -58,3 +58,5 @@
 logo.bin       │  58  58     let value_58 = compute(58);
 src/           │  59  59     let value_59 = compute(59);
▌  lib.rs       │  60  60     let value_60 = compute(60);
                │▎     61     let token_61 = parse_token(input, 61);
                │▎     62     let value_62 = compute(62);
                │~
                │~
  NORMAL    @19b4e9ae  src/lib.rs [RO]                                14%  1:1
```

The hunk view (`Space g p`) is the same block without the sidebar, named for
its file:

```
──────── src/lib.rs ───────────────────────────────────────────────────── +1 -1
   ⋮   ⋮ @@ -27,7 +27,7 @@
  27  27     let value_27 = compute(27);
  28  28     let value_28 = compute(28);
  29  29     let value_29 = compute(29);
▎ 30         let value_30 = compute(30);
▎     30     let thirty = compute(30);
  31  31     let value_31 = compute(31);
```

A binary file:

```
 docs/          │──────── logo.bin ───────────────────────────────────── binary
   new-name.md  │~
▌logo.bin       │~
```

- Row 0 is a heading (rule, no band); `+N` / `-M` in the diff inks; `binary`
  `fg.muted` (D1, D3).
- `@@` rows are breaks: band from the pane's first column (after the sidebar's
  `│`) to the last content column; `⋮` at the last digit of each number column.
- Lines are unchanged: `▎`, numbers, tint, emphasis, syntax. The hunk view gets
  the same syntax colour as the commit diff (D2).
- Buffer text: `src/lib.rs` (today `src/lib.rs +2 -0`, and `hunk +1 -1`), then
  as today.
- At 48 columns without the sidebar: `──────── …ender/statusline/layout.rs ─── +12 -3`.

### 4.6 Commit message

The grid does not change. Three inks do:

```
   1 surfaces: one visual language
   2
   3 # Write the message above; lines starting with # are left out.
```

- Line 1 is bold `fg.text`. Cells past column 72 of line 1 are `diag.warning`.
- A non-empty line 2 is `diag.warning`.
- `#` lines are `syn.comment` italic (M1).

This is the buffer's syntax, read from its text as any language's is. It is a
file; it gets no headings, rules or virtual text.

## 5. Rejected

- **A full box** (`│` down both sides). It costs four cells, the right side
  cannot hold beside lines that scroll or run long, and a border around text
  you edit reads as a widget. 0088's cards are floating things on a raised
  plane; a buffer is not one.
- **Keeping the corners** and adding a blank row. Still a box with no sides.
- **A heading on a band** (Zed's file bar). The band is already the hunk
  header's and the gap's. Two levels on one plane is D1 again. A rule for the
  block and a band for the jump keeps them apart with what 0088 already has.
- **A dotted rule for breaks, no band.** Considered so that background would
  only ever mean state. In a long diff the band is what lets you find the next
  hunk; a faint dotted line between syntax-coloured rows is too weak.
- **Indenting nested rows in the buffer text** (two spaces before each diff
  line in status). `/^+` would stop finding additions and a yank would stop
  being a patch (0096 §2.1). The rail does the job without touching the text.
- **A rail on every block** (collection, diff). The number gutter is already
  there and the rule already runs through it; a second vertical mark is noise.
- **Line numbers kept on list surfaces** "because Vim has them". They number
  rows of a list, not lines of a file, and the modeline already shows the row.
- **Fold arrows** (`▸` / `▾`) on status files. 0088 rule 5 removed them
  everywhere; the rail shows what is open.
- **Chips for refs or status letters.** Keycaps are the only chip (rule 6).
- **A colour per author or a new lane palette in the log.** No new colours.
- **Unicode graph lines** (`│ ├╮ ╯`) drawn over git's `* | \ /`. It needs the
  row above and below to choose a glyph, and the painted cell would not be the
  buffer's character. Worth doing only from a typed graph (§7), not by
  re-reading ASCII art.
- **Counts kept in the buffer text** (`Staged (3)`, `── 3 hits`). Then a count
  sits in one of two places depending on the surface, and a yanked heading
  carries a number that was true when it was copied.
- **Drawing headings at the pane's first column** (name in the gutter). The
  caret would sit five cells right of the letters it is on.
- **Changing the review buffer** (`Space R`), which prints a raw unified patch
  (`--- a/…`, `+++ b/…`). It should become a block per file with the same diff
  lines, but it is 0094's surface with its own keys and row model; it is
  deferred to keep this change reviewable (§8).

## 6. Decoration, implementation and what to delete

Nothing below changes a key, a motion, an operator, a job or a cache. The
write-through of the collection (kernel `excerpt::source_edit`) is untouched:
it maps body bytes, and body bytes do not move relative to their excerpt.

### 6.1 One renderer: `crates/strop/src/render/block/`

One component, one directory, files by concern (AGENTS.md):

| File | Holds |
|---|---|
| `mod.rs` | `enum Row<'a> { Title, Heading(Heading<'a>), Break(Break<'a>), Entry(Entry<'a>), DiffLine(&'a DiffLine, …), Blank }` and `fn paint(row, geometry, tokens) -> Painted { gutter, text, fact, row_bg }` |
| `heading.rs` | the heading: the part table of §3.9 and its one-pass allocator, then spans. Pure: `fn layout(parts, cells) -> Grants` |
| `gutter.rs` | the four gutters of §3.5 as data: number cells, `⋮`, rule, rail, `▌` |
| `diff_line.rs` | one changed-line painter: sign, tint, emphasis, prefix. Used by the diff view and by status |
| `tests.rs` (or `tests/`) | §9 |

Each surface gets a small adapter that turns its typed row into a `block::Row`
— data in, data out, no drawing:

| Surface | Adapter reads | Lives in |
|---|---|---|
| collection | `CollectionRowInfo` (gains `hits: usize` on heading rows) | `render/buffer/rows/collection.rs` |
| status | `StatusRow`, `StatusView` | `render/diff/status.rs` |
| log, file history | `LogRow` | `render/diff/list.rs` |
| commit files | `ChangedFile`, sha | `render/diff/list.rs` |
| diff, hunk | `DiffRow` | `render/diff.rs` |
| Search's group heading | location, hit count | `render/picker/rows/search.rs` calls `block::heading` with a 1-cell gutter and no rule in it |

`render/buffer/rows.rs::render_pane` then asks one question per row — "who
owns this row?" — and gets a typed owner (`File`, `Help`, `Directory`,
`Review`, `Block(Row)`), computed as data the way `editor/dispatch.rs` does,
instead of today's ordered chain of `review_row` / `diff_row` /
`collection_row_kind` / `directory::row` / `surface_list_row` / `help`. That
function is most of the 534-line file; this is the moment to split it:
`rows/{mod,owner,overlays,collection}.rs`.

The heading allocator has the same shape as the modeline's
(`statusline/layout.rs::allot`). If the two tables fit one function without
bending either, move the pass to `render/allot.rs` and have both use it. If
they do not, keep two small passes — no generic machinery for two callers.

### 6.2 Engine: text builders and geometry

| File | Change |
|---|---|
| `strop-engine/src/editor/collections/projection.rs` | heading text = the path; gap text = `N lines not shown`; no `╰` row; one empty row before each heading. `CollectionRow` becomes `Title, Blank, Heading(usize), Gap, Body` (`CardTop`, `CardBottom` deleted, every caller migrated). `CollectionRowInfo` gains `hits`. |
| `strop-engine/src/editor/collections/{quickfix,navigation,write,view_positions}.rs` | follow the renamed roles. `Enter` on a heading still opens the file (0097). An edit touching a heading, gap or blank row is still refused. |
| `strop-engine/src/editor/git_work/rows.rs` | section text = `Staged` (count is `StatusView::count`, already there). |
| `strop-engine/src/editor/git_memory/mod.rs` (`diff_surface_text`), `presentation.rs` (`label`) | row 0 text = the label only; the hunk preview passes the file's repo-relative path as its label instead of `hunk`. **0101 is working in this directory** — these are two text builders, not the job path; land them after 0101 or hand them to its author. |
| `strop-engine/src/editor/git_memory/file_list.rs` | drop the empty second row; row index → file index shifts by one, in one place. Same note. |
| `strop-engine/src/editor/prepare/geometry.rs` | `enum Gutter { Lines, Source, Pair, List }` and `fn gutter(doc) -> (Gutter, width)`; `number_gutter_width` and `left_inset` read it. The renderer reads the same value, so the caret and the paint cannot disagree. |
| `strop-engine/src/editor/git_work/porcelain.rs` | none. The commit buffer's three inks are render-side (`is_commit_message` exists). |

### 6.3 Delete

- `collection_card_top_spans` and the `CardTop` / `Gap` / `CardBottom` arms in
  `buffer/rows.rs`.
- `diff::structural_row`; `diff/status.rs`'s per-row styling (kept: the
  adapter); `diff/list.rs::file_row_spans` and the `ChangedFiles` arm of
  `surface_list_row`; `RowDecoration` once every caller goes through
  `block::Painted`.
- `picker/rows/search.rs::file_header`'s own layout (kept: a call into
  `block::heading`).
- `diff_digits` in `buffer/rows.rs` (the gutter is data now).
- The words `· modified` / `· read-only`.
- No shims: `CollectionRow::CardTop` does not stay as an alias.

### 6.4 Order of work

Each step ends green (`fmt`, `clippy -D warnings`, `test --locked`) and is its
own commit.

1. `block/heading.rs` with its table and tests; Search's heading calls it.
   **The `grep-*` goldens must not change** — that is the proof the merge is
   faithful.
2. `Gutter` in the engine; `block/gutter.rs`; list gutter on status, log,
   files. Regenerate `git-status-*`.
3. Collection: new text and roles in the engine, adapter, cursor line.
   Regenerate `collected-*`, `collection-*`; update the engine's collection
   tests for the new text.
4. `block/diff_line.rs`; status uses it; rail; section headings.
5. Diff and hunk: heading row, break rows, full-width bands, hunk label.
6. Commit files with today's data; log step A.
7. Commit message inks.
8. Row owner as data; split `buffer/rows.rs`.
9. New goldens (§9), `CHANGELOG.md` `## Unreleased`, `docs/search.md` (the
   "one card per file" paragraph and its picture), `docs/git.md` (status,
   log, commit view), `plans/STATUS.md` (this plan moves to "in force"),
   0097 §3.4 and 0096 §2.1 amended where they quote the old rows.
10. `docker compose run --build --rm test`.

Log step B and the fuller commit view (§4.3, §4.4) follow when the data
exists.

## 7. Data this needs from the job side

Not changed by this plan; asked of whoever owns the git job path (0101).

| Wanted | For | Today |
|---|---|---|
| typed log rows: graph, sha, ref names, subject, author, commit time | §4.3 step B: aligned columns, refs, a stable age | `LogRow { text, sha }`, the age already a sentence |
| a commit's subject, body, author and time with its file list | §4.4: a commit viewer that shows the commit | sha only |
| per-file status letter, old path for a rename, a binary flag | §4.4: `R old → new`, `binary` | path and two counts; binary is `+0 -0` |
| per-entry `+N -M` in the status snapshot | a fact on status file rows | absent; not drawn in §4.2 |

## 8. Deferred

- The review buffer (`Space R`) as blocks.
- The directory buffer and the help buffer: they already have their own
  settled look and were not in the note.
- The echo row can cover a heading when the cursor is on the pane's last row
  (seen while rendering: `hit 11 of 27` hid `src/a.rs`'s heading). It is the
  echo row's placement, not a surface's drawing.
- Hiding the diff's file sidebar on narrow panes.

## 9. Test contract

Goldens (`render/golden/`), `hone` and `vellum`, at **48×16, 80×24, 120×40 and
200×50** for the surfaces of this plan (the existing scenes keep their two
sizes):

| Golden | Scene |
|---|---|
| `collected-*` (regenerate) | straight after `Ctrl-O`, cursor on the second hit |
| `collection-*` (regenerate) | after an edit: `●` on the heading |
| `collection-blocks-*` (new) | §1's scene: long path, block boundary, adjacent hits, hits on the first and last line, a read-only file, cursor on a heading |
| `git-status-*` (regenerate) | two files expanded, one binary, cursor in the second hunk |
| `git-log-*`, `git-log-file-*` (new) | a merge, a long subject; rows injected through a fixture so the age is fixed |
| `git-files-*` (new) | three files, one of them a 90-cell path |
| `git-diff-*`, `git-diff-binary-*` (new) | two hunks with the sidebar; a binary file |
| `git-hunk-*` (new) | `Space g p` on a changed line |
| `git-commit-*` (regenerate) | an 80-cell subject, so the warning ink shows |
| `grep-*` | **unchanged** |

Pure functions:

- The heading allocator: every cell of the row is accounted for; no part gets
  less than its floor or more than it wants; narrowing the row, parts leave in
  table order; it fits at every width from 1 to 200 with names of 1 to 200
  cells, wide characters included.
- The gutter: each kind paints exactly its width for every row kind.

Properties over painted frames, for every surface, both themes, widths 20–200:

- **Caret truth:** for every row, the painted cells from the text column begin
  with the buffer line's own text (clipped headings excepted, §3.8); whatever
  follows is padding or a fact.
- **Rules line up:** every heading's rule starts at the pane's first column,
  and every heading's last painted cell is in the same column.
- **Bands are whole:** a break row has one background from the pane's first
  column to its last content column.
- **Separation:** exactly one blank row before each heading but the first row
  of a buffer; none after a heading.
- **One accent:** on a log, status or files frame without search hits, accent
  cells are only the cursor mark and the branch you are on.
- No row is cut without `…`.

Behaviour (engine):

- `yy` on a collection heading yanks the path; on a gap, `12 lines not shown`;
  a visual yank over a block contains no box-drawing character.
- `yy` on a status section yanks `Staged`; a yank of an expanded hunk still
  applies with `git apply`.
- 0097's flow tests and 0094's write tests pass with the new text: `]q`,
  `[q`, `Enter` on a heading, `:cdo`, `:w`, `u`, refused edits on heading, gap
  and blank rows.
- 0096's status keys act on the same rows as before (`s`, `u`, `-`, `=`, `X`,
  `]c`, `cc`).
- The caret column on a heading, a break and a list row equals the engine's
  `left_inset` + the buffer column, for all four gutters.

## 10. For the maintainer to choose

These are taste. The plan takes the first option in each.

1. **Line numbers on status, log and commit files:** drop them (§3.5), or keep
   the five-cell gutter for the sake of sameness with file buffers.
2. **A jump inside a block:** band (§3.1), or a dotted rule and no band.
3. **The heading's rule:** through the gutter to the pane's first column
   (§3.2), or starting at the text column like today's card.
4. **What a yank of a heading copies:** the path alone (§3.8), or path and
   count as now.
5. **The rail** under an expanded status file: keep it, or rely on bands and
   tints.
6. **The log's accent and lanes:** sha in grey with coloured lanes as today,
   or lanes in grey too.
7. **Author and age in the log** (step B): render-only at the right, so a
   yanked row is `sha subject`; or in the buffer text as columns.
8. **The 72-column warning** in the commit message: on, or leave the buffer
   alone.

### Decided (maintainer, 2026-11-12)

1 drop the numbers; 2 band; 3 the rule runs full width; 7 author and age
render-only at the right (step B — the typed log rows of §7 are in scope
for 0.64). The rest take the plan's first option: 4 the path alone, 5 keep
the rail, 6 sha grey with coloured lanes, 8 the 72-column warning on.

## 11. Implementation record

Shipped as §6.4 orders it, one commit per step: the heading and its table
(`render/block/heading.rs`; Search's group header calls it and the `grep-*`
goldens did not change); `Gutter` in `prepare/geometry.rs` and
`render/block/gutter.rs`; the collection as blocks of words; the status
buffer through `render/block/diff_line.rs` with the rail and section
headings; the diff and hunk views; commit files and log step A; the commit
message's inks; the row owner as data and `buffer/rows/{mod,owner,overlays,
collection,commit_message}.rs`. The goldens of §9 and its frame properties
(`render/block_frame_tests.rs`, every width from 20 to 200, both themes)
are in.

Departures, with reasons:

- **A file with no line changes says `no line changes`, not `binary`.**
  Today's data has no binary flag (numstat's `-` lands as 0/0, a native diff
  of a binary file has no hunks), so a binary file, a mode change and a pure
  rename look the same. Saying `binary` would be wrong for two of them. The
  flag is one of §7's requests; `git-diff-binary-*` will change with it.
- **The accent is not a graph lane.** Decision 6 keeps coloured lanes, but
  the first lane was the accent, which breaks §9's one-accent property on
  every log frame. The lanes keep five of their colours and take
  `syn.number` for the sixth.
- **A heading keeps its rule on the cursor row.** On the list surfaces the
  cursor row's `▌` would take the rule's first cell, breaking "every
  heading's rule starts at the pane's first column". The cursor line's band
  says where the cursor is; the rule stays. On every surface the cursor row's
  band wins over a break's band, as the status buffer already did.
- **The heading table has one more part**: a remote file's `ssh:host`, whole,
  after the marks (§3.9 does not list it; 0088 keeps the authority whole).
- **`+N -M` is not bold**, in a heading's fact and in a row's; the diff inks
  carry it.
- **The section the cursor is in** has its rule in `fg.muted` in the status
  buffer too, as §3.2 says for any block.
- **The collection's gutter** is as wide as the largest *source* line number
  it shows, not the view's row count.
- **The line history's buffer** is `git log · <file>:<start>-<end>`, beside
  the file history's `git log · <file>`.
- **Search at narrow widths**: following the table, the leader now outranks
  the directory, so a tight group header cuts the directory from the left
  (`…s/remote.md ─── 3`) before it drops the leader; the gap below a leader
  is two cells, not one.
- **Goldens**: the existing scenes (`collected`, `collection`, `git-status`,
  `git-commit`) keep their two sizes; the new ones (`collection-blocks`,
  `git-log`, `git-log-file`, `git-files`, `git-diff`, `git-diff-binary`,
  `git-hunk`) are at the four sizes, from a repository built with fixed
  names and dates.
- **Found on the way**: `plans/STATUS.md` carried a stray merge marker
  (`||||||| 476f9d9`); removed with this plan's move to "in force".
