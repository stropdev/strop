# 0095 — The keys

- Status: **in force**
- Date: 2026-11-10
- Adopts: [0091](0091-editor-critique-0.56.md) §2 and §7 "the keys";
  [0092](0092-registers-and-pinned-project.md) §3
- Amends: [0001](0001-vision-and-design.md) §5.10 (the differential corpus),
  [0016](0016-one-input-machine.md) (the walker gains a Visual table),
  [0003](0003-modes-leader-and-house-style.md) §5.7 (the Missing list)

The owner's direction: "if at the end we are Neovim reimplemented in Rust that
should be fine." Neovim 0.9.5 with no configuration (`-u NONE`) is the
specification. Where Neovim's default differs from Vim's, Neovim's wins
(`Y` is `y$`, Visual `*` searches the selection). The target is a day of work
without a key doing something else.

0091 §2 measured 151 everyday sequences: 80 differed. The differential harness
that is supposed to find such things had 72 cases. Both are fixed here: the
keys, and the harness that keeps them fixed.

## 1. What is wrong, by cause

Reading 0091 §2 and 0092 §3 against the source gives five causes, not eighty.

1. **The vocabulary is small.** `strop-grammar` knows five operators, twenty
   motions and four text objects. `gU`, `ip`, `'a` are not wrong; they do not
   parse.
2. **The resolver lacks three of Vim's range rules.** `:help exclusive-linewise`
   (an exclusive motion ending in column 1 gives the line break back), the `w`
   rule (the last word moved over ends at its line), and "a motion that cannot
   move fails" (`dk` on line 1). Each is one place after motion resolution.
3. **Counts repeat the command instead of parameterising it.** `3J`, `3x`,
   `5~`, `2D` ran the single command N times: one join too many, N register
   writes, `$` never left the line.
4. **Visual mode is a second, smaller editor.** Its keys are a hand-written
   `match` in `visual.rs` (`d y c x > <`); everything else fell into the
   grammar and was refused. It needs the table Normal has.
5. **The `:` line is a `match` on whole strings.** No abbreviations, no
   ranges beyond numbers, a dozen commands.

## 2. Design

### 2.1 Grammar (`strop-grammar`)

The resolver stays pure and stays the one consumer-facing function
(doctrine 2). It grows by data.

- **Operators**: `gU` `gu` `g~` `g?` `=` `gq` `gw` `!` join `d y c > <`.
  `Op::from_keys(&str)` replaces `from_key(char)`; a doubled operator
  (`gUU`, `gUgU`, `==`, `g??`) is `Target::Linewise` as `dd` is.
- **Motions**: `-` `+` `Enter` `(` `)` `[(` `[{` `])` `]}` `g_` `g0` `gj`
  `gk` `N%`, counted `$` (down N−1 lines), and three that need editor state:
  marks (`'a` `` `a `` and the special marks `'[ '] '< '> '' '. '^`), the
  window lines (`H` `M` `L`) and the repeats (`;` `,` `n` `N`).
- **Text objects**: `ip` `ap`, `is` `as`, `it` `at`, `ib` `iB` (aliases of
  `i(` `i{`), `gn` `gN`; counts on all of them (`d2aw`, `2i(` for the second
  enclosing pair). In Visual, an object extends a selection that is already
  more than one character (`viwiw`).
- **Forcing**: `v` `V` `Ctrl-V` between operator and motion
  (`Command::force`), as `:help o_v` defines.
- **Editor-bound targets.** A mark, a window line or "the last search" is not
  in the buffer, so the resolver cannot see it. The parser emits the unbound
  form (`Motion::Mark { name, exact }`); the engine binds it to a position
  immediately before resolution (`Editor::bind_command`), and dot-repeat keeps
  the unbound command so `.` reads the mark again. A command that reaches the
  resolver unbound resolves to nothing. No callback, no second resolver.
- **Range rules**, applied once after the motion resolves, in Vim's order:
  the `w` rule, then exclusive-linewise, then forcing.
- **A motion that cannot move fails**: `j` on the last line, `k` on the
  first, `h` in column 1 and `l` at the line's last character under an
  operator resolve to nothing; the command is refused and the register is not
  touched.

### 2.2 One operator engine

`normal/execute.rs` applies a resolved plan; `visual.rs` had its own copy for
selections. They become one function, `apply_operator(op, register, ranges)`,
with Normal passing the plan's ranges and Visual passing the selection (one
range, or one per row for a block). The new operators are arms of it, so every
operator works in Normal, in all three Visual modes, with a register and under
dot-repeat without a second implementation.

### 2.3 Visual has a table

The binding table gets a second compiled index: rows whose sections include
`visual` are Visual's keys, looked up before the grammar. `U u ~ J gJ r p P I
A o O * # = gq D X Y C R Ctrl-A Ctrl-X` are rows with typed handlers that
receive the count and the register, as Normal's do. Rows that are Visual-only
(`u` lowercases there) are left out of Normal's index, so one key can mean two
things in two modes without the first row in the table winning.

Dot-repeat after a Visual operator replays it over the same extent from the
cursor, as Vim does: N lines for a linewise selection, N characters on one
line, otherwise the selection's line and column deltas.

### 2.4 Normal and Insert

- `x` `X` are `dl` `dh` and `D C Y` are `d$ c$ y$`: counts, registers and dot
  come from the grammar instead of a loop.
- `R` is an Insert session that overtypes; Backspace restores what it
  replaced. `Ctrl-A` / `Ctrl-X` find the number at or after the cursor
  (decimal, `0x`, `0b`; a leading `-`) as Neovim's default `nrformats` does.
- `J` `gJ` take a count of lines; `~` a count of characters; `&` `g&` `@:`
  repeat the last substitute or command line; `gp` `gP` `]p` `[p`; `g*` `g#`;
  `gi` `gv` unchanged.
- Insert: `Ctrl-W` (word back), `Ctrl-U` (to the start of the inserted text
  or the line), `Ctrl-O` (one Normal command), `Ctrl-T` / `Ctrl-D` (indent),
  `Ctrl-H`.
- Marks move with the text (they were raw offsets and went stale on the
  first edit above them).

### 2.5 Registers (0092 §3)

Visual `p` replaces the selection and leaves the replaced text in `""`; `P`
leaves `""` alone. `"#` is the alternate file. Fields (pickers, prompts)
write registers when they yank or delete. Appending to a blockwise register
re-pads its rows.

### 2.6 The `:` line

`editor/ex/` replaces `normal/ex.rs`: a range parser (`N . $ % 'a '< /pat/
?pat? +N -N`, `,` and `;`), a command table with Vim's abbreviations and
`!`, and one function per command. The table is the source for completion and
for the dispatch — one list, as `EX_COMMANDS` was meant to be.

- Ranges are honoured by every command that takes one; `:` from Visual is
  prefilled with `'<,'>` and returns to Normal.
- `:s`: `&` `\0`–`\9` `~` `\n` `\r` `\t` and `\u \l \U \L \E` in the
  replacement; flags `g i I n c &`; `:&` `:&&` `:~`; case-sensitive unless
  `\c`. `c` asks at each match with the match highlighted (`y n a q l`).
- `:g` `:v` (and `:g!`), `:norm[al]`, `:m`, `:t`/`:co`, `:d [x]`, `:y [x]`,
  `:pu`, `:j`, `:>` `:<`, `:sort`, `:[range]!cmd`.
- `:b {n|name}` `:bn` `:bp` `:bd` `:ls`, `:reg`, `:marks`, `:wa` `:wqa`
  `:xa` `:x` `:up`, `:cd` `:pwd`.
- **`:norm` is a function**, `Editor::ex_normal(range, keys)`, so the change
  track's `:cdo norm …` composes with it. This plan builds no quickfix.
- **`:cd` is the one way the project root changes** (0092 §2.3 pins it).
  It re-anchors what is keyed by `Editor::cwd`: relative buffer names, the
  session and recovery files, picker catalogues, the LSP "outside the
  project" test.

`/` keeps its case policy; that belongs to the find track (0093).

### 2.7 What is still missing is listed

`keymap/missing.rs` holds two lists: **Missing** (Neovim binds it, strop does
not: folds, tabs, `q:`, `"=`, …) and **Reassigned** (Neovim binds it, strop
uses the key for something else: `Q`, `K`, `gI`, `gs`, Visual `S`, …).
`strop compat` and `docs/vim-compat.md` print both. A test feeds every
Missing row and asserts it does not dispatch — a row that starts working
fails the test until it is moved into the table.

## 3. The corpus

`crates/strop/tests/differential/` replaces the one file:

| File | Cases | Source |
|---|---|---|
| `hand.rs` | 95 | behaviours somebody once got wrong (the old list) |
| `critique.rs` | 151 | 0091 §2, verbatim, on its ten-line buffer |
| `ex.rs` | written | ranges, `:s`, `:g`, `:norm`, registers, buffers |
| `generated.rs` | generated | operator × target × count × mode |

**The generator.** A case is `position + operator + target`, rendered three
ways: Normal (`{op}{count}{target}`), Visual (`v{count}{target}{op}`) and
dot-repeat (the Normal case, a move, then `.`). Operators that open Insert
type a marker and Escape; `y` is followed by `P` so the yank is visible in
the text as well as in the registers. Targets are every motion and text
object of §2.1 that does not need a second buffer or a window size. Two
fixtures (prose with paragraphs and sentences; code with nesting, quotes,
tags and numbers) and a fixed set of start positions (line start, mid-word,
on punctuation, on a blank line, last line). The names are derived
(`gen/code@3:4/d2aw/visual`), so a failure names its own reproduction.

The compared state is the final text, cursor, mode, `""`, `"0 "1 "2 "- "a
"b` and `modified`. Cases run one process pair each, on every core.

**Divergence policy.**

- A divergence is a bug until it has a reason. `divergences::KNOWN` is capped
  at 10 entries; an entry is one case or one named family (a prefix) and
  says what differs and why strop keeps it.
- An entry whose cases all agree again fails the run until it is delisted.
- A generated combination that Neovim itself refuses or that depends on
  state the harness does not set (window height, a second buffer, the
  clipboard) is not generated; it is not a divergence.
- The floor is in the test: fewer than `MINIMUM_CASES` (600) fails.

## 4. Tests beyond the corpus

- Engine unit tests for what the harness cannot see: Replace mode's
  Backspace, `Ctrl-O`, register contents after Visual `p` / `P`, marks moving
  with edits, `:cd` re-anchoring, `:s///c` answers, `:ls` / `:reg` / `:marks`
  text.
- The keymap tests gain the Visual index and the Missing assertion.
- A cell-grid golden for the `:` line prefilled with `'<,'>` from Visual.

## 5. Order of work

1. Harness split and the critique cases (the baseline: 77 of 246 diverge).
2. Grammar vocabulary and range rules; the generator.
3. One operator engine; the Visual table.
4. Normal, Insert and register items.
5. The `:` line.
6. Missing/Reassigned lists and `strop compat`.
7. The tagline (§6).

## 6. The tagline

0091 §3.3: "see the cut before you make it" is true on four surfaces and
false on the grammar. Once the corpus is green it is replaced, everywhere the
product says it (welcome card, `--version`, `install.sh`, crate descriptions,
README), by **"Vim's keys. Nothing to configure."** The website is a separate
change.

## 7. Not in this plan

Folds, tabs, `:set` beyond what the editor already has options for, a
`[keys]` remap, quickfix and `:cdo` (0094), `/` smartcase (0093).
